//! Server state management.

use slopcoder_core::{
    agent::AgentConfig,
    environment::EnvironmentConfig,
    task::{Task, TaskId, TaskStore},
    CodexEvent,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

struct AppStateInner {
    /// Environment configuration.
    config: EnvironmentConfig,
    /// Task storage.
    tasks: TaskStore,
    /// Event broadcasters for each running task.
    event_channels: HashMap<TaskId, broadcast::Sender<CodexEvent>>,
    /// Agent configuration.
    agent_config: AgentConfig,
    /// Path to environments config file.
    config_path: PathBuf,
}

impl AppState {
    /// Create new application state from config file path.
    pub async fn new(config_path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let config = EnvironmentConfig::load(&config_path).await?;

        Ok(Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                config,
                tasks: TaskStore::new(),
                event_channels: HashMap::new(),
                agent_config: AgentConfig::default(),
                config_path,
            })),
        })
    }

    /// Create state with a given config (for testing).
    pub fn with_config(config: EnvironmentConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                config,
                tasks: TaskStore::new(),
                event_channels: HashMap::new(),
                agent_config: AgentConfig::default(),
                config_path: PathBuf::new(),
            })),
        }
    }

    /// Reload environment configuration from disk.
    pub async fn reload_config(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.inner.write().await;
        if inner.config_path.as_os_str().is_empty() {
            return Ok(());
        }
        inner.config = EnvironmentConfig::load(&inner.config_path).await?;
        Ok(())
    }

    /// Get a clone of the environment config.
    pub async fn get_config(&self) -> EnvironmentConfig {
        self.inner.read().await.config.clone()
    }

    /// Get the agent config.
    pub async fn get_agent_config(&self) -> AgentConfig {
        self.inner.read().await.agent_config.clone()
    }

    /// List all tasks.
    pub async fn list_tasks(&self) -> Vec<Task> {
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

    /// Insert a new task.
    pub async fn insert_task(&self, task: Task) {
        self.inner.write().await.tasks.insert(task);
    }

    /// Update a task's session ID.
    pub async fn set_task_session_id(&self, id: TaskId, session_id: Uuid) {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.session_id = Some(session_id);
        }
    }

    /// Start a run on a task.
    pub async fn start_task_run(&self, id: TaskId, prompt: String) -> bool {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            if task.can_run() {
                task.start_run(prompt);
                return true;
            }
        }
        false
    }

    /// Complete a task run.
    pub async fn complete_task_run(&self, id: TaskId, success: bool) {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.complete_run(success);
        }
        // Clean up the event channel
        inner.event_channels.remove(&id);
    }

    /// Create an event broadcaster for a task.
    pub async fn create_event_channel(&self, id: TaskId) -> broadcast::Sender<CodexEvent> {
        let (tx, _) = broadcast::channel(100);
        self.inner
            .write()
            .await
            .event_channels
            .insert(id, tx.clone());
        tx
    }

    /// Subscribe to events for a task.
    pub async fn subscribe_to_task(&self, id: TaskId) -> Option<broadcast::Receiver<CodexEvent>> {
        self.inner
            .read()
            .await
            .event_channels
            .get(&id)
            .map(|tx| tx.subscribe())
    }

    /// Broadcast an event for a task.
    pub async fn broadcast_event(&self, id: TaskId, event: CodexEvent) {
        if let Some(tx) = self.inner.read().await.event_channels.get(&id) {
            let _ = tx.send(event);
        }
    }
}
