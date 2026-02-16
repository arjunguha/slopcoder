//! Shared RPC protocol between slopcoder-server and slopagent.

use crate::{
    anyagent::AgentKind,
    environment::Environment,
    task::{Task, TaskId},
    AgentEvent,
};
use serde::{Deserialize, Serialize};

/// Message envelope exchanged over the coordinator<->agent websocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEnvelope {
    /// Initial hello sent by the agent immediately after connecting.
    Hello {
        hostname: String,
        #[serde(default)]
        display_name: Option<String>,
    },
    /// Request sent by the coordinator.
    Request {
        request_id: String,
        request: AgentRequest,
    },
    /// Response sent by the agent.
    Response {
        request_id: String,
        response: AgentResponse,
    },
    /// Error response sent by the agent.
    Error {
        request_id: String,
        status: u16,
        error: String,
    },
    /// Event emitted by a running task.
    TaskEvent { task_id: TaskId, event: AgentEvent },
}

/// Request payloads from coordinator -> agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentRequest {
    ListEnvironments,
    CreateEnvironment { name: String },
    ListBranches { environment: String },
    ListTasks,
    GetTask { task_id: TaskId },
    CreateTask { request: AgentCreateTaskRequest },
    SendPrompt { task_id: TaskId, prompt: String },
    GetTaskOutput { task_id: TaskId },
    GetTaskDiff { task_id: TaskId },
    InterruptTask { task_id: TaskId },
    MergeTask { task_id: TaskId },
    GetMergeReadiness { task_id: TaskId },
    ArchiveTask { task_id: TaskId },
    DeleteTask { task_id: TaskId, force: bool },
}

/// Response payloads from agent -> coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentResponse {
    Environments {
        environments: Vec<Environment>,
    },
    Environment {
        environment: Environment,
    },
    Branches {
        branches: Vec<String>,
    },
    Tasks {
        tasks: Vec<Task>,
    },
    Task {
        task: Option<Task>,
    },
    CreatedTask {
        id: TaskId,
        worktree_path: String,
    },
    TaskOutput {
        events: Vec<AgentEvent>,
    },
    TaskDiff {
        staged: String,
        unstaged: String,
    },
    MergeResult {
        status: String,
        message: String,
    },
    MergeReadiness {
        can_merge: bool,
        reason: Option<String>,
    },
    ArchiveResult {
        status: String,
        message: String,
    },
    DeleteResult {
        status: String,
        message: String,
    },
    Ack,
}

/// Task creation payload from coordinator -> agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCreateTaskRequest {
    pub environment: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub use_worktree: bool,
    #[serde(default)]
    pub web_search: bool,
    pub prompt: String,
    #[serde(default)]
    pub agent: Option<AgentKind>,
}
