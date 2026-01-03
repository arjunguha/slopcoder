//! Agent wrapper for the Codex CLI.
//!
//! This module provides an async interface for spawning and managing
//! Codex CLI processes, including streaming JSONL output.

use crate::events::CodexEvent;
use futures::Stream;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Errors that can occur when running the agent.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("Failed to spawn codex process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("Failed to parse event: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("Agent process exited with error: {0}")]
    ProcessError(String),

    #[error("No session ID received from agent")]
    NoSessionId,
}

/// Configuration for running the agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Path to the codex binary.
    pub codex_path: String,
    /// Model to use (optional, uses codex default if not set).
    pub model: Option<String>,
    /// Additional flags to pass to codex.
    pub extra_args: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            codex_path: "codex".to_string(),
            model: None,
            extra_args: Vec::new(),
        }
    }
}

/// Result of running the agent.
#[derive(Debug)]
pub struct AgentResult {
    /// The session ID from this run.
    pub session_id: Uuid,
    /// Whether the agent completed successfully.
    pub success: bool,
    /// Exit code if available.
    pub exit_code: Option<i32>,
}

/// A running agent process with streaming output.
pub struct Agent {
    child: Child,
    event_rx: mpsc::Receiver<Result<CodexEvent, AgentError>>,
    session_id: Option<Uuid>,
}

impl Agent {
    /// Spawn a new agent for a fresh task.
    pub async fn spawn(
        config: &AgentConfig,
        working_dir: &Path,
        prompt: &str,
    ) -> Result<Self, AgentError> {
        let mut cmd = Command::new(&config.codex_path);

        cmd.arg("exec")
            .arg("--json")
            .arg("--dangerously-bypass-approvals-and-sandbox")
            .arg("-C")
            .arg(working_dir);

        if let Some(model) = &config.model {
            cmd.arg("-m").arg(model);
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        cmd.arg(prompt);

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null());

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let event_rx = Self::spawn_reader(stdout);

        Ok(Self {
            child,
            event_rx,
            session_id: None,
        })
    }

    /// Spawn an agent to resume an existing session.
    pub async fn resume(
        config: &AgentConfig,
        working_dir: &Path,
        session_id: Uuid,
        prompt: &str,
    ) -> Result<Self, AgentError> {
        let mut cmd = Command::new(&config.codex_path);

        cmd.arg("exec")
            .arg("--json")
            .arg("--dangerously-bypass-approvals-and-sandbox")
            .arg("-C")
            .arg(working_dir);

        if let Some(model) = &config.model {
            cmd.arg("-m").arg(model);
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        cmd.arg("resume")
            .arg(session_id.to_string())
            .arg(prompt);

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null());

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let event_rx = Self::spawn_reader(stdout);

        Ok(Self {
            child,
            event_rx,
            session_id: Some(session_id),
        })
    }

    /// Spawn a background task to read lines from stdout and parse events.
    fn spawn_reader(
        stdout: tokio::process::ChildStdout,
    ) -> mpsc::Receiver<Result<CodexEvent, AgentError>> {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                let event = CodexEvent::parse(&line).map_err(AgentError::from);
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        rx
    }

    /// Get the session ID if we've received it.
    pub fn session_id(&self) -> Option<Uuid> {
        self.session_id
    }

    /// Wait for the next event from the agent.
    pub async fn next_event(&mut self) -> Option<Result<CodexEvent, AgentError>> {
        let result = self.event_rx.recv().await?;

        // Extract session ID from session_meta event
        if let Ok(event) = &result {
            if let Some(id) = event.session_id() {
                self.session_id = Some(id);
            }
        }

        Some(result)
    }

    /// Wait for the agent to complete and return the result.
    pub async fn wait(mut self) -> Result<AgentResult, AgentError> {
        // Drain remaining events to ensure we have the session ID
        while let Some(result) = self.next_event().await {
            if let Err(e) = result {
                tracing::warn!("Error reading event: {}", e);
            }
        }

        let status = self.child.wait().await?;

        let session_id = self.session_id.ok_or(AgentError::NoSessionId)?;

        Ok(AgentResult {
            session_id,
            success: status.success(),
            exit_code: status.code(),
        })
    }

    /// Kill the agent process.
    pub async fn kill(&mut self) -> Result<(), AgentError> {
        self.child.kill().await.map_err(AgentError::from)
    }

    /// Check if the agent is still running.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, AgentError> {
        self.child.try_wait().map_err(AgentError::from)
    }
}

/// Stream wrapper for agent events.
pub struct AgentEventStream {
    agent: Agent,
}

impl AgentEventStream {
    /// Create a new event stream from an agent.
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }

    /// Get the session ID if available.
    pub fn session_id(&self) -> Option<Uuid> {
        self.agent.session_id()
    }

    /// Consume the stream and return the underlying agent.
    pub fn into_agent(self) -> Agent {
        self.agent
    }
}

impl Stream for AgentEventStream {
    type Item = Result<CodexEvent, AgentError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.agent.event_rx).poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.codex_path, "codex");
        assert!(config.model.is_none());
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn test_agent_config_with_model() {
        let config = AgentConfig {
            codex_path: "/usr/bin/codex".to_string(),
            model: Some("gpt-4".to_string()),
            extra_args: vec!["--verbose".to_string()],
        };
        assert_eq!(config.model, Some("gpt-4".to_string()));
    }
}
