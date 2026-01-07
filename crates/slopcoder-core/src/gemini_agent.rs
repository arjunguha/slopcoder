//! Gemini agent wrapper for the Gemini CLI.
//!
//! This module provides an async interface for spawning and managing
//! Gemini CLI processes, including streaming JSONL output.

use crate::anyagent::{AgentError, AgentResult, AnyAgent, GeminiAgentConfig};
use crate::events::AgentEvent;
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

/// A running Gemini agent process with streaming output.
pub struct GeminiAgent {
    child: Child,
    event_rx: mpsc::Receiver<Result<AgentEvent, AgentError>>,
    session_id: Option<Uuid>,
}

impl GeminiAgent {
    /// Spawn a new agent for a fresh task.
    pub async fn spawn(
        config: &GeminiAgentConfig,
        working_dir: &Path,
        prompt: &str,
    ) -> Result<Self, AgentError> {
        let mut cmd = Command::new(&config.gemini_path);

        // Basic flags for automation and JSON output
        cmd.arg("--output-format")
            .arg("stream-json")
            .arg("--approval-mode")
            .arg("yolo")
            .current_dir(working_dir);

        if let Some(model) = &config.model {
            cmd.arg("--model").arg(model);
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        // Positional prompt
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
        config: &GeminiAgentConfig,
        working_dir: &Path,
        session_id: Uuid,
        prompt: &str,
    ) -> Result<Self, AgentError> {
        let mut cmd = Command::new(&config.gemini_path);

        cmd.arg("--output-format")
            .arg("stream-json")
            .arg("--approval-mode")
            .arg("yolo")
            .arg("--resume")
            .arg(session_id.to_string())
            .current_dir(working_dir);

        if let Some(model) = &config.model {
            cmd.arg("--model").arg(model);
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

                // Parse Gemini events
                let events = AgentEvent::parse_gemini(&line).map_err(AgentError::from);
                match events {
                    Ok(events) => {
                        for event in events {
                            if tx.send(Ok(event)).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        if tx.send(Err(err)).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        rx
    }
}

#[async_trait]
impl AnyAgent for GeminiAgent {
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
