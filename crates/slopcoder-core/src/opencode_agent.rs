//! OpenCode agent wrapper for the OpenCode CLI.
//!
//! This module provides an async interface for spawning and managing
//! OpenCode CLI processes, including streaming JSONL output.

use crate::anyagent::{AgentError, AgentResult, AnyAgent, OpencodeAgentConfig};
use crate::events::AgentEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

/// File name for storing session ID mappings (UUID -> original session string).
const SESSION_MAP_FILE: &str = ".opencode-sessions.json";

/// A running OpenCode agent process with streaming output.
pub struct OpencodeAgent {
    child: Child,
    event_rx: mpsc::Receiver<Result<AgentEvent, AgentError>>,
    session_id: Option<Uuid>,
    /// Original session string from opencode (e.g., "ses_xxx").
    session_string: Option<String>,
    /// Working directory for storing session mappings.
    working_dir: std::path::PathBuf,
}

impl OpencodeAgent {
    /// Spawn a new agent for a fresh task.
    pub async fn spawn(
        config: &OpencodeAgentConfig,
        working_dir: &Path,
        prompt: &str,
        _web_search: bool,
    ) -> Result<Self, AgentError> {
        let mut cmd = Command::new(&config.opencode_path);

        cmd.arg("run")
            .arg("--format")
            .arg("json")
            .arg("--model")
            .arg(&config.model)
            .current_dir(working_dir);

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
            session_string: None,
            working_dir: working_dir.to_path_buf(),
        })
    }

    /// Spawn an agent to resume an existing session.
    pub async fn resume(
        config: &OpencodeAgentConfig,
        working_dir: &Path,
        session_id: Uuid,
        prompt: &str,
        _web_search: bool,
    ) -> Result<Self, AgentError> {
        // Look up the original session string from the mapping file
        let session_string = Self::load_session_string(working_dir, &session_id).await?;

        let mut cmd = Command::new(&config.opencode_path);

        cmd.arg("run")
            .arg("--format")
            .arg("json")
            .arg("--model")
            .arg(&config.model)
            .arg("--session")
            .arg(&session_string)
            .current_dir(working_dir);

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
            session_string: Some(session_string),
            working_dir: working_dir.to_path_buf(),
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

                // Parse opencode format
                let events = AgentEvent::parse_opencode(&line).map_err(AgentError::from);
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

    /// Generate a deterministic UUID from an opencode session string.
    fn session_string_to_uuid(session_string: &str) -> Uuid {
        // Use UUID v5 (SHA-1 based) with a custom namespace
        let namespace = Uuid::NAMESPACE_OID;
        Uuid::new_v5(&namespace, session_string.as_bytes())
    }

    /// Save the session string mapping to a file.
    async fn save_session_mapping(&self) -> Result<(), AgentError> {
        if let (Some(uuid), Some(session_str)) = (&self.session_id, &self.session_string) {
            let map_path = self.working_dir.join(SESSION_MAP_FILE);

            // Load existing mappings or create new
            let mut mappings: HashMap<String, String> = if map_path.exists() {
                let content = tokio::fs::read_to_string(&map_path).await?;
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                HashMap::new()
            };

            // Add our mapping
            mappings.insert(uuid.to_string(), session_str.clone());

            // Save back
            let content =
                serde_json::to_string_pretty(&mappings).map_err(|e| AgentError::ParseError(e))?;
            tokio::fs::write(&map_path, content).await?;
        }
        Ok(())
    }

    /// Load the session string from the mapping file.
    async fn load_session_string(working_dir: &Path, uuid: &Uuid) -> Result<String, AgentError> {
        let map_path = working_dir.join(SESSION_MAP_FILE);

        if !map_path.exists() {
            return Err(AgentError::ProcessError(format!(
                "Session mapping file not found: {:?}",
                map_path
            )));
        }

        let content = tokio::fs::read_to_string(&map_path).await?;
        let mappings: HashMap<String, String> =
            serde_json::from_str(&content).map_err(AgentError::ParseError)?;

        mappings.get(&uuid.to_string()).cloned().ok_or_else(|| {
            AgentError::ProcessError(format!("Session ID {} not found in mapping", uuid))
        })
    }
}

#[async_trait]
impl AnyAgent for OpencodeAgent {
    async fn next_event(&mut self) -> Option<Result<AgentEvent, AgentError>> {
        let result = self.event_rx.recv().await?;

        if let Ok(event) = &result {
            // Check for standard session ID (from SessionStarted event)
            if let Some(id) = event.session_id() {
                self.session_id = Some(id);
            }
            // Check for OpenCode-specific session ID with original string
            // This is needed to store the mapping for resume functionality
            if let Some((uuid, session_str)) = event.opencode_session_id() {
                self.session_id = Some(uuid);
                self.session_string = Some(session_str);
                // Save the mapping for future resume
                if let Err(e) = self.save_session_mapping().await {
                    tracing::warn!("Failed to save session mapping: {}", e);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opencode_agent_config_with_model() {
        let config = OpencodeAgentConfig {
            opencode_path: "/usr/bin/opencode".to_string(),
            model: "test-model".to_string(),
            extra_args: vec!["--force".to_string()],
        };
        assert_eq!(config.model, "test-model");
    }

    #[test]
    fn test_session_string_to_uuid_deterministic() {
        let session_str = "ses_4723d5c64ffeo3VMtIbToaB7GI";
        let uuid1 = OpencodeAgent::session_string_to_uuid(session_str);
        let uuid2 = OpencodeAgent::session_string_to_uuid(session_str);
        assert_eq!(uuid1, uuid2);
    }

    #[test]
    fn test_session_string_to_uuid_different_inputs() {
        let uuid1 = OpencodeAgent::session_string_to_uuid("ses_abc123");
        let uuid2 = OpencodeAgent::session_string_to_uuid("ses_def456");
        assert_ne!(uuid1, uuid2);
    }
}
