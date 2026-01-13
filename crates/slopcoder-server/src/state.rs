//! Server state management.

use slopcoder_core::{
    anyagent::AnyAgentConfig,
    environment::{EnvironmentConfig, EnvironmentError},
    persistence::PersistentTaskStore,
    task::{Task, TaskId},
    AgentEvent, PersistenceError,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// Errors that can occur in state operations.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Task not found: {0}")]
    TaskNotFound(TaskId),

    #[error("Task worktree no longer exists: {0}")]
    WorktreeMissing(TaskId),

    #[error("Task cannot accept prompts in current state")]
    TaskNotReady,

    #[error("Persistence error: {0}")]
    PersistenceError(#[from] PersistenceError),
}

/// Errors that can occur during startup validation.
#[derive(Debug, Error)]
pub enum StartupError {
    #[error("New environments directory validation failed: {0}")]
    NewEnvDirValidation(EnvironmentError),

    #[error("Environment '{name}' failed validation: {source}")]
    EnvironmentValidation {
        name: String,
        source: EnvironmentError,
    },

    #[error("Environment '{name}' failed to list branches: {source}")]
    EnvironmentBranches {
        name: String,
        source: EnvironmentError,
    },
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

struct AppStateInner {
    /// Environment configuration.
    config: EnvironmentConfig,
    /// Persistent task storage.
    tasks: PersistentTaskStore,
    /// Event broadcasters for each running task.
    event_channels: HashMap<TaskId, broadcast::Sender<AgentEvent>>,
    /// Interrupt senders for each running task.
    interrupt_channels: HashMap<TaskId, tokio::sync::oneshot::Sender<()>>,
    /// Agent configuration.
    agent_config: AnyAgentConfig,
    /// Model name for feature branch generation.
    branch_model: String,
    /// Optional password for API access.
    auth_password: Option<String>,
    /// Path to environments config file.
    #[allow(dead_code)]
    config_path: PathBuf,
}

impl AppState {
    /// Create new application state from config file path.
    /// Loads existing tasks from environment directories.
    pub async fn new(
        config_path: PathBuf,
        branch_model: String,
        auth_password: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = EnvironmentConfig::load(&config_path).await?;

        // Validate new environments directory exists and is a directory
        if let Err(err) = config.validate_new_environments_directory().await {
            return Err(Box::new(StartupError::NewEnvDirValidation(err)));
        }

        for env in &config.environments {
            if let Err(err) = env.validate().await {
                return Err(Box::new(StartupError::EnvironmentValidation {
                    name: env.name.clone(),
                    source: err,
                }));
            }

            if let Err(err) = env.list_branches().await {
                return Err(Box::new(StartupError::EnvironmentBranches {
                    name: env.name.clone(),
                    source: err,
                }));
            }
        }

        // Create persistent store and register all environments
        let mut tasks = PersistentTaskStore::new();
        for env in &config.environments {
            tasks.register_environment(env.name.clone(), env.directory.clone());
        }

        // Load existing tasks from disk
        tasks.load_all().await?;

        let task_count = tasks.list().len();
        if task_count > 0 {
            tracing::info!("Loaded {} existing tasks from disk", task_count);
        }

        Ok(Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                config,
                tasks,
                event_channels: HashMap::new(),
                interrupt_channels: HashMap::new(),
                agent_config: AnyAgentConfig::default(),
                branch_model,
                auth_password,
                config_path,
            })),
        })
    }

    /// Create state with a given config (for testing).
    #[allow(dead_code)]
    pub fn with_config(config: EnvironmentConfig) -> Self {
        let mut tasks = PersistentTaskStore::new();
        for env in &config.environments {
            tasks.register_environment(env.name.clone(), env.directory.clone());
        }

        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                config,
                tasks,
                event_channels: HashMap::new(),
                interrupt_channels: HashMap::new(),
                agent_config: AnyAgentConfig::default(),
                branch_model: "claude-haiku-4-5".to_string(),
                auth_password: None,
                config_path: PathBuf::new(),
            })),
        }
    }

    /// Reload environment configuration from disk.
    #[allow(dead_code)]
    pub async fn reload_config(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.inner.write().await;
        if inner.config_path.as_os_str().is_empty() {
            return Ok(());
        }
        inner.config = EnvironmentConfig::load(&inner.config_path).await?;

        // Re-register environments in case new ones were added
        // Collect first to avoid borrow conflict
        let envs: Vec<_> = inner
            .config
            .environments
            .iter()
            .map(|e| (e.name.clone(), e.directory.clone()))
            .collect();

        for (name, directory) in envs {
            inner.tasks.register_environment(name, directory);
        }

        Ok(())
    }

    /// Get a clone of the environment config.
    pub async fn get_config(&self) -> EnvironmentConfig {
        self.inner.read().await.config.clone()
    }

    /// Get the directory for a named environment.
    pub async fn get_environment_directory(&self, name: &str) -> Option<PathBuf> {
        self.inner
            .read()
            .await
            .config
            .find(name)
            .map(|env| env.directory.clone())
    }

    /// Get the agent config.
    pub async fn get_agent_config(&self) -> AnyAgentConfig {
        self.inner.read().await.agent_config.clone()
    }

    /// Get the model used for branch name generation.
    pub async fn get_branch_model(&self) -> String {
        self.inner.read().await.branch_model.clone()
    }

    /// Get the configured API password, if any.
    pub async fn get_auth_password(&self) -> Option<String> {
        self.inner.read().await.auth_password.clone()
    }

    /// List all tasks, cleaning up any with missing worktrees.
    pub async fn list_tasks(&self) -> Vec<Task> {
        // First, cleanup stale tasks (requires write lock)
        {
            let mut inner = self.inner.write().await;
            if let Err(e) = inner.tasks.cleanup_stale_tasks().await {
                tracing::warn!("Failed to cleanup stale tasks: {}", e);
            }
        }

        // Then return the list (read lock)
        self.inner
            .read()
            .await
            .tasks
            .list()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Get a task by ID.
    pub async fn get_task(&self, id: TaskId) -> Option<Task> {
        self.inner.read().await.tasks.get(id).cloned()
    }

    /// Check if a task's worktree still exists.
    pub async fn validate_task_worktree(&self, id: TaskId) -> bool {
        self.inner.read().await.tasks.validate_task_worktree(id)
    }

    /// Insert a new task (persists to disk).
    pub async fn insert_task(&self, task: Task) -> Result<(), StateError> {
        self.inner.write().await.tasks.insert(task).await?;
        Ok(())
    }

    /// Update a task's session ID (persists to disk).
    pub async fn set_task_session_id(&self, id: TaskId, session_id: Uuid) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.session_id = Some(session_id);
            inner.tasks.save_task(id).await?;
            Ok(())
        } else {
            Err(StateError::TaskNotFound(id))
        }
    }

    /// Start a run on a task (persists to disk).
    /// Validates that the worktree still exists before starting.
    pub async fn start_task_run(&self, id: TaskId, prompt: String) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;

        // Check worktree exists
        if !inner.tasks.validate_task_worktree(id) {
            return Err(StateError::WorktreeMissing(id));
        }

        if let Some(task) = inner.tasks.get_mut(id) {
            if task.can_run() {
                task.start_run(prompt);
                inner.tasks.save_task(id).await?;
                Ok(())
            } else {
                Err(StateError::TaskNotReady)
            }
        } else {
            Err(StateError::TaskNotFound(id))
        }
    }

    /// Complete a task run (persists to disk).
    pub async fn complete_task_run(&self, id: TaskId, success: bool) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.complete_run(success);
            inner.tasks.save_task(id).await?;
        }
        // Clean up the channels
        inner.event_channels.remove(&id);
        inner.interrupt_channels.remove(&id);
        Ok(())
    }

    /// Interrupt a task run (persists to disk).
    pub async fn interrupt_task_run(&self, id: TaskId) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            if !task.is_running() {
                return Err(StateError::TaskNotReady);
            }
            task.interrupt_run();
            inner.tasks.save_task(id).await?;
        } else {
            return Err(StateError::TaskNotFound(id));
        }
        // Clean up channels
        inner.event_channels.remove(&id);
        inner.interrupt_channels.remove(&id);
        Ok(())
    }

    /// Register an interrupt channel for a task. Returns a receiver to listen for interrupts.
    pub async fn register_interrupt_channel(&self, id: TaskId) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner.write().await.interrupt_channels.insert(id, tx);
        rx
    }

    /// Send an interrupt signal to a task.
    pub async fn send_interrupt(&self, id: TaskId) -> bool {
        if let Some(tx) = self.inner.write().await.interrupt_channels.remove(&id) {
            tx.send(()).is_ok()
        } else {
            false
        }
    }

    /// Create an event broadcaster for a task.
    pub async fn create_event_channel(&self, id: TaskId) -> broadcast::Sender<AgentEvent> {
        let (tx, _) = broadcast::channel(100);
        self.inner
            .write()
            .await
            .event_channels
            .insert(id, tx.clone());
        tx
    }

    /// Subscribe to events for a task.
    pub async fn subscribe_to_task(&self, id: TaskId) -> Option<broadcast::Receiver<AgentEvent>> {
        self.inner
            .read()
            .await
            .event_channels
            .get(&id)
            .map(|tx| tx.subscribe())
    }

    /// Broadcast an event for a task.
    pub async fn broadcast_event(&self, id: TaskId, event: AgentEvent) {
        if let Some(tx) = self.inner.read().await.event_channels.get(&id) {
            let _ = tx.send(event);
        }
    }

    /// Create a new environment with a fresh git repository.
    pub async fn create_new_environment(
        &self,
        name: &str,
    ) -> Result<slopcoder_core::environment::Environment, EnvironmentError> {
        let mut inner = self.inner.write().await;

        let env = inner.config.initialize_new_environment(name).await?;

        // Register the new environment with the task store
        inner.tasks.register_environment(env.name.clone(), env.directory.clone());

        // Save the updated config to disk
        if !inner.config_path.as_os_str().is_empty() {
            inner.config.save(&inner.config_path).await?;
        }

        Ok(env)
    }
}
