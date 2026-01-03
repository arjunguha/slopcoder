//! Task management for agent runs.
//!
//! A task represents a single agent session running in a worktree,
//! along with its execution history and prompts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub Uuid);

impl TaskId {
    /// Create a new random task ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Status of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task created but agent not yet started.
    Pending,
    /// Agent is currently running.
    Running,
    /// Agent completed successfully.
    Completed,
    /// Agent failed with an error.
    Failed,
}

/// A single prompt and its result in the task history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRun {
    /// The prompt text sent to the agent.
    pub prompt: String,
    /// When this prompt was sent.
    pub started_at: DateTime<Utc>,
    /// When the agent finished (if finished).
    pub finished_at: Option<DateTime<Utc>>,
    /// Whether this run succeeded.
    pub success: Option<bool>,
}

impl PromptRun {
    /// Create a new prompt run starting now.
    pub fn new(prompt: String) -> Self {
        Self {
            prompt,
            started_at: Utc::now(),
            finished_at: None,
            success: None,
        }
    }

    /// Mark this run as finished.
    pub fn finish(&mut self, success: bool) {
        self.finished_at = Some(Utc::now());
        self.success = Some(success);
    }
}

/// A task representing an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task identifier.
    pub id: TaskId,
    /// Name of the environment this task belongs to.
    pub environment: String,
    /// Base branch this task was created from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Feature branch this task is working on.
    #[serde(default, alias = "branch")]
    pub feature_branch: String,
    /// Path to the worktree directory.
    pub worktree_path: PathBuf,
    /// Current status of the task.
    pub status: TaskStatus,
    /// Codex session ID (set after first run).
    pub session_id: Option<Uuid>,
    /// When the task was created.
    pub created_at: DateTime<Utc>,
    /// History of prompt runs.
    pub history: Vec<PromptRun>,
}

impl Task {
    /// Create a new task.
    pub fn new(
        environment: String,
        base_branch: Option<String>,
        feature_branch: String,
        worktree_path: PathBuf,
    ) -> Self {
        Self {
            id: TaskId::new(),
            environment,
            base_branch,
            feature_branch,
            worktree_path,
            status: TaskStatus::Pending,
            session_id: None,
            created_at: Utc::now(),
            history: Vec::new(),
        }
    }

    /// Check if this task can accept new prompts.
    pub fn can_run(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Pending | TaskStatus::Completed | TaskStatus::Failed
        )
    }

    /// Check if the agent is currently running.
    pub fn is_running(&self) -> bool {
        self.status == TaskStatus::Running
    }

    /// Start a new prompt run.
    pub fn start_run(&mut self, prompt: String) {
        self.status = TaskStatus::Running;
        self.history.push(PromptRun::new(prompt));
    }

    /// Mark the current run as completed.
    pub fn complete_run(&mut self, success: bool) {
        if let Some(run) = self.history.last_mut() {
            run.finish(success);
        }
        self.status = if success {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
    }

    /// Get the last prompt that was run.
    pub fn last_prompt(&self) -> Option<&str> {
        self.history.last().map(|r| r.prompt.as_str())
    }
}

/// In-memory storage for tasks.
#[derive(Debug, Default)]
pub struct TaskStore {
    tasks: std::collections::HashMap<TaskId, Task>,
}

impl TaskStore {
    /// Create a new empty task store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a task into the store.
    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    /// Get a task by ID.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Get a mutable reference to a task by ID.
    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    /// List all tasks.
    pub fn list(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }

    /// List tasks for a specific environment.
    pub fn list_by_environment(&self, environment: &str) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| t.environment == environment)
            .collect()
    }

    /// Remove a task.
    pub fn remove(&mut self, id: TaskId) -> Option<Task> {
        self.tasks.remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_task_creation() {
        let task = Task::new(
            "my-env".to_string(),
            Some("main".to_string()),
            "feature/test".to_string(),
            PathBuf::from("/tmp/worktree"),
        );

        assert_eq!(task.base_branch.as_deref(), Some("main"));
        assert_eq!(task.feature_branch, "feature/test");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.session_id.is_none());
        assert!(task.history.is_empty());
        assert!(task.can_run());
    }

    #[test]
    fn test_task_run_lifecycle() {
        let mut task = Task::new(
            "env".to_string(),
            Some("main".to_string()),
            "feature/one".to_string(),
            PathBuf::from("/tmp"),
        );

        assert!(task.can_run());
        assert!(!task.is_running());

        task.start_run("Hello world".to_string());
        assert!(!task.can_run());
        assert!(task.is_running());
        assert_eq!(task.history.len(), 1);

        task.complete_run(true);
        assert!(task.can_run());
        assert!(!task.is_running());
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.history[0].success, Some(true));
    }

    #[test]
    fn test_task_store() {
        let mut store = TaskStore::new();

        let task1 = Task::new(
            "env1".to_string(),
            Some("main".to_string()),
            "feature/a".to_string(),
            PathBuf::from("/tmp/1"),
        );
        let task2 = Task::new(
            "env2".to_string(),
            Some("main".to_string()),
            "feature/b".to_string(),
            PathBuf::from("/tmp/2"),
        );

        let id1 = task1.id;
        let id2 = task2.id;

        store.insert(task1);
        store.insert(task2);

        assert_eq!(store.list().len(), 2);
        assert!(store.get(id1).is_some());
        assert!(store.get(id2).is_some());

        assert_eq!(store.list_by_environment("env1").len(), 1);
    }
}
