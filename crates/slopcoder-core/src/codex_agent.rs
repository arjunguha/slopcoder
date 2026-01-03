//! Codex agent wrapper for the Codex CLI.
//!
//! This module provides an async interface for spawning and managing
//! Codex CLI processes, including streaming JSONL output.

use crate::anyagent::{AgentError, AgentResult, AnyAgent, CodexAgentConfig};
use crate::events::AgentEvent;
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

/// A running Codex agent process with streaming output.
pub struct CodexAgent {
    child: Child,
    event_rx: mpsc::Receiver<Result<AgentEvent, AgentError>>,
    session_id: Option<Uuid>,
}

impl CodexAgent {
    /// Spawn a new agent for a fresh task.
    pub async fn spawn(
        config: &CodexAgentConfig,
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
        config: &CodexAgentConfig,
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
    ) -> mpsc::Receiver<Result<AgentEvent, AgentError>> {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                let event = AgentEvent::parse_codex(&line).map_err(AgentError::from);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_agent_config_with_model() {
        let config = CodexAgentConfig {
            codex_path: "/usr/bin/codex".to_string(),
            model: Some("gpt-4".to_string()),
            extra_args: vec!["--verbose".to_string()],
        };
        assert_eq!(config.model, Some("gpt-4".to_string()));
    }
}

#[async_trait]
impl AnyAgent for CodexAgent {
    async fn next_event(&mut self) -> Option<Result<AgentEvent, AgentError>> {
        let result = self.event_rx.recv().await?;

        if let Ok(event) = &result {
            if let Some(id) = event.session_id() {
                self.session_id = Some(id);
            }
        }

        Some(result)
    }

    async fn wait(mut self: Box<Self>) -> Result<AgentResult, AgentError> {
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

    async fn kill(&mut self) -> Result<(), AgentError> {
        self.child.kill().await.map_err(AgentError::from)
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, AgentError> {
        self.child.try_wait().map_err(AgentError::from)
    }

    fn session_id(&self) -> Option<Uuid> {
        self.session_id
    }
}
