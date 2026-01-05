//! Agent-agnostic abstractions and configuration.

use crate::claude_agent::ClaudeAgent;
use crate::codex_agent::CodexAgent;
use crate::cursor_agent::CursorAgent;
use crate::opencode_agent::OpencodeAgent;
use crate::events::AgentEvent;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur when running an agent.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("Failed to spawn agent process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("Failed to parse event: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("Agent process exited with error: {0}")]
    ProcessError(String),

    #[error("No session ID received from agent")]
    NoSessionId,
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

/// Which agent implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    Claude,
    Cursor,
    Opencode,
}

impl Default for AgentKind {
    fn default() -> Self {
        AgentKind::Codex
    }
}

/// Configuration for running the Codex agent.
#[derive(Debug, Clone)]
pub struct CodexAgentConfig {
    /// Path to the codex binary.
    pub codex_path: String,
    /// Model to use (optional, uses codex default if not set).
    pub model: Option<String>,
    /// Additional flags to pass to codex.
    pub extra_args: Vec<String>,
}

impl Default for CodexAgentConfig {
    fn default() -> Self {
        Self {
            codex_path: "codex".to_string(),
            model: None,
            extra_args: Vec::new(),
        }
    }
}

/// Configuration for running the Claude agent.
#[derive(Debug, Clone)]
pub struct ClaudeAgentConfig {
    /// Path to the claude binary.
    pub claude_path: String,
    /// Model to use (optional, uses claude default if not set).
    pub model: Option<String>,
    /// Additional flags to pass to claude.
    pub extra_args: Vec<String>,
}

impl Default for ClaudeAgentConfig {
    fn default() -> Self {
        Self {
            claude_path: "claude".to_string(),
            model: None,
            extra_args: Vec::new(),
        }
    }
}

/// Configuration for running the Cursor agent.
#[derive(Debug, Clone)]
pub struct CursorAgentConfig {
    /// Path to the cursor-agent binary.
    pub cursor_path: String,
    /// Model to use (optional, uses cursor-agent default if not set).
    pub model: Option<String>,
    /// Additional flags to pass to cursor-agent.
    pub extra_args: Vec<String>,
}

impl Default for CursorAgentConfig {
    fn default() -> Self {
        Self {
            cursor_path: "cursor-agent".to_string(),
            model: None,
            extra_args: Vec::new(),
        }
    }
}

/// Configuration for running the OpenCode agent.
#[derive(Debug, Clone)]
pub struct OpencodeAgentConfig {
    /// Path to the opencode binary.
    pub opencode_path: String,
    /// Model to use (hard-coded for opencode).
    pub model: String,
    /// Additional flags to pass to opencode.
    pub extra_args: Vec<String>,
}

impl Default for OpencodeAgentConfig {
    fn default() -> Self {
        Self {
            opencode_path: "opencode".to_string(),
            model: "litellm-guha-anderson/boa".to_string(),
            extra_args: Vec::new(),
        }
    }
}

/// Combined configuration for all supported agents.
#[derive(Debug, Clone)]
pub struct AnyAgentConfig {
    pub codex: CodexAgentConfig,
    pub claude: ClaudeAgentConfig,
    pub cursor: CursorAgentConfig,
    pub opencode: OpencodeAgentConfig,
}

impl Default for AnyAgentConfig {
    fn default() -> Self {
        Self {
            codex: CodexAgentConfig::default(),
            claude: ClaudeAgentConfig::default(),
            cursor: CursorAgentConfig::default(),
            opencode: OpencodeAgentConfig::default(),
        }
    }
}

/// A running agent process with streaming output.
#[async_trait]
pub trait AnyAgent: Send {
    /// Wait for the next event from the agent.
    async fn next_event(&mut self) -> Option<Result<AgentEvent, AgentError>>;
    /// Wait for the agent to complete and return the result.
    async fn wait(self: Box<Self>) -> Result<AgentResult, AgentError>;
    /// Kill the agent process.
    async fn kill(&mut self) -> Result<(), AgentError>;
    /// Check if the agent is still running.
    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, AgentError>;
    /// Get the session ID if available.
    fn session_id(&self) -> Option<Uuid>;
}

/// Spawn a new agent for a fresh task.
pub async fn spawn_anyagent(
    kind: AgentKind,
    config: &AnyAgentConfig,
    working_dir: &Path,
    prompt: &str,
) -> Result<Box<dyn AnyAgent>, AgentError> {
    match kind {
        AgentKind::Codex => {
            let agent = CodexAgent::spawn(&config.codex, working_dir, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Claude => {
            let agent = ClaudeAgent::spawn(&config.claude, working_dir, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Cursor => {
            let agent = CursorAgent::spawn(&config.cursor, working_dir, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Opencode => {
            let agent = OpencodeAgent::spawn(&config.opencode, working_dir, prompt).await?;
            Ok(Box::new(agent))
        }
    }
}

/// Spawn an agent to resume an existing session.
pub async fn resume_anyagent(
    kind: AgentKind,
    config: &AnyAgentConfig,
    working_dir: &Path,
    session_id: Uuid,
    prompt: &str,
) -> Result<Box<dyn AnyAgent>, AgentError> {
    match kind {
        AgentKind::Codex => {
            let agent = CodexAgent::resume(&config.codex, working_dir, session_id, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Claude => {
            let agent = ClaudeAgent::resume(&config.claude, working_dir, session_id, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Cursor => {
            let agent = CursorAgent::resume(&config.cursor, working_dir, session_id, prompt).await?;
            Ok(Box::new(agent))
        }
        AgentKind::Opencode => {
            let agent = OpencodeAgent::resume(&config.opencode, working_dir, session_id, prompt).await?;
            Ok(Box::new(agent))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_kind_default() {
        assert_eq!(AgentKind::default(), AgentKind::Codex);
    }

    #[test]
    fn test_codex_config_default() {
        let config = CodexAgentConfig::default();
        assert_eq!(config.codex_path, "codex");
        assert!(config.model.is_none());
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn test_claude_config_default() {
        let config = ClaudeAgentConfig::default();
        assert_eq!(config.claude_path, "claude");
        assert!(config.model.is_none());
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn test_cursor_config_default() {
        let config = CursorAgentConfig::default();
        assert_eq!(config.cursor_path, "cursor-agent");
        assert!(config.model.is_none());
        assert!(config.extra_args.is_empty());
    }

    #[test]
    fn test_opencode_config_default() {
        let config = OpencodeAgentConfig::default();
        assert_eq!(config.opencode_path, "opencode");
        assert_eq!(config.model, "litellm-guha-anderson/boa");
        assert!(config.extra_args.is_empty());
    }
}
