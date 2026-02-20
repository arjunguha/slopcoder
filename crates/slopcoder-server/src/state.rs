//! Coordinator state for connected slopagents.

use chrono::{DateTime, Utc};
use slopcoder_core::{
    agent_rpc::{AgentEnvelope, AgentRequest, AgentResponse},
    environment::Environment,
    task::{Task, TaskId},
    AgentEvent,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("Host must be specified when multiple agents are connected")]
    HostRequired,

    #[error("Host not connected: {0}")]
    HostNotConnected(String),

    #[error("No agents are connected")]
    NoAgentsConnected,

    #[error("Agent disconnected")]
    AgentDisconnected,

    #[error("Agent request timed out")]
    AgentTimeout,

    #[error("Remote error ({status}): {error}")]
    RemoteError { status: u16, error: String },
}

#[derive(Debug, Clone)]
pub struct ConnectedAgent {
    pub id: Uuid,
    pub host: String,
    pub hostname: String,
    pub connected_at: DateTime<Utc>,
    outbound_tx: mpsc::UnboundedSender<AgentEnvelope>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<PendingResponse>>>>,
}

type PendingResponse = Result<AgentResponse, RemoteError>;

#[derive(Debug, Clone)]
pub struct RemoteError {
    pub status: u16,
    pub error: String,
}

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Data(Vec<u8>),
    Closed,
    Error(String),
}

impl ConnectedAgent {
    pub async fn request(&self, request: AgentRequest) -> Result<AgentResponse, StateError> {
        self.request_with_timeout(request, Duration::from_secs(120))
            .await
    }

    pub async fn request_with_timeout(
        &self,
        request: AgentRequest,
        timeout_duration: Duration,
    ) -> Result<AgentResponse, StateError> {
        let request_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel::<PendingResponse>();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        if self
            .outbound_tx
            .send(AgentEnvelope::Request {
                request_id: request_id.clone(),
                request,
            })
            .is_err()
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&request_id);
            return Err(StateError::AgentDisconnected);
        }

        let result = match timeout(timeout_duration, rx).await {
            Ok(result) => result,
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&request_id);
                return Err(StateError::AgentTimeout);
            }
        };

        let result = match result {
            Ok(result) => result,
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&request_id);
                return Err(StateError::AgentDisconnected);
            }
        };

        match result {
            Ok(response) => Ok(response),
            Err(err) => Err(StateError::RemoteError {
                status: err.status,
                error: err.error,
            }),
        }
    }

    pub fn send_envelope(&self, envelope: AgentEnvelope) -> Result<(), StateError> {
        self.outbound_tx
            .send(envelope)
            .map_err(|_| StateError::AgentDisconnected)
    }
}

#[derive(Debug, Clone)]
pub struct HostInfo {
    pub host: String,
    pub hostname: String,
    pub connected_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

struct AppStateInner {
    ui_auth_password: Option<String>,
    agent_auth_password: String,
    agents_by_id: HashMap<Uuid, ConnectedAgent>,
    host_to_id: HashMap<String, Uuid>,
    task_hosts: HashMap<TaskId, String>,
    environment_cache_by_host: HashMap<String, Vec<Environment>>,
    task_cache_by_host: HashMap<String, Vec<Task>>,
    list_backoff_until: HashMap<String, Instant>,
    event_channels: HashMap<TaskId, broadcast::Sender<AgentEvent>>,
    terminal_channels: HashMap<Uuid, broadcast::Sender<TerminalEvent>>,
}

impl AppState {
    pub fn new(ui_auth_password: Option<String>, agent_auth_password: String) -> Self {
        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                ui_auth_password,
                agent_auth_password,
                agents_by_id: HashMap::new(),
                host_to_id: HashMap::new(),
                task_hosts: HashMap::new(),
                environment_cache_by_host: HashMap::new(),
                task_cache_by_host: HashMap::new(),
                list_backoff_until: HashMap::new(),
                event_channels: HashMap::new(),
                terminal_channels: HashMap::new(),
            })),
        }
    }

    pub async fn get_ui_auth_password(&self) -> Option<String> {
        self.inner.read().await.ui_auth_password.clone()
    }

    pub async fn get_agent_auth_password(&self) -> String {
        self.inner.read().await.agent_auth_password.clone()
    }

    pub async fn register_agent(
        &self,
        hostname: String,
        display_name: Option<String>,
        outbound_tx: mpsc::UnboundedSender<AgentEnvelope>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<PendingResponse>>>>,
    ) -> ConnectedAgent {
        let mut inner = self.inner.write().await;

        let base = display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&hostname)
            .to_string();
        let host = unique_host_label(&base, &inner.host_to_id);

        let agent = ConnectedAgent {
            id: Uuid::new_v4(),
            host: host.clone(),
            hostname,
            connected_at: Utc::now(),
            outbound_tx,
            pending,
        };

        inner.host_to_id.insert(host.clone(), agent.id);
        inner.agents_by_id.insert(agent.id, agent.clone());
        agent
    }

    pub async fn unregister_agent(&self, agent_id: Uuid) {
        let mut inner = self.inner.write().await;
        let Some(agent) = inner.agents_by_id.remove(&agent_id) else {
            return;
        };

        inner.host_to_id.remove(&agent.host);
        inner.task_hosts.retain(|_, host| host != &agent.host);
        inner.environment_cache_by_host.remove(&agent.host);
        inner.task_cache_by_host.remove(&agent.host);
        inner.list_backoff_until.remove(&agent.host);
        tracing::info!("Agent '{}' disconnected", agent.host);
    }

    pub async fn list_hosts(&self) -> Vec<HostInfo> {
        let mut hosts: Vec<_> = self
            .inner
            .read()
            .await
            .agents_by_id
            .values()
            .map(|agent| HostInfo {
                host: agent.host.clone(),
                hostname: agent.hostname.clone(),
                connected_at: agent.connected_at,
            })
            .collect();
        hosts.sort_by(|a, b| a.host.cmp(&b.host));
        hosts
    }

    pub async fn list_agents(&self) -> Vec<ConnectedAgent> {
        self.inner
            .read()
            .await
            .agents_by_id
            .values()
            .cloned()
            .collect()
    }

    pub async fn get_agent_for_host(&self, host: &str) -> Option<ConnectedAgent> {
        let inner = self.inner.read().await;
        let id = inner.host_to_id.get(host)?;
        inner.agents_by_id.get(id).cloned()
    }

    pub async fn get_host_for_task(&self, task_id: TaskId) -> Option<String> {
        self.inner.read().await.task_hosts.get(&task_id).cloned()
    }

    pub async fn set_task_host(&self, task_id: TaskId, host: String) {
        self.inner.write().await.task_hosts.insert(task_id, host);
    }

    pub async fn clear_task_host(&self, task_id: TaskId) {
        self.inner.write().await.task_hosts.remove(&task_id);
    }

    pub async fn record_tasks_for_host(&self, host: &str, tasks: &[Task]) {
        let mut inner = self.inner.write().await;
        for task in tasks {
            inner.task_hosts.insert(task.id, host.to_string());
        }
    }

    pub async fn cache_environments_for_host(&self, host: &str, environments: &[Environment]) {
        self.inner
            .write()
            .await
            .environment_cache_by_host
            .insert(host.to_string(), environments.to_vec());
    }

    pub async fn get_cached_environments_for_host(&self, host: &str) -> Vec<Environment> {
        self.inner
            .read()
            .await
            .environment_cache_by_host
            .get(host)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn cache_tasks_for_host(&self, host: &str, tasks: &[Task]) {
        self.inner
            .write()
            .await
            .task_cache_by_host
            .insert(host.to_string(), tasks.to_vec());
    }

    pub async fn get_cached_tasks_for_host(&self, host: &str) -> Vec<Task> {
        self.inner
            .read()
            .await
            .task_cache_by_host
            .get(host)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn should_skip_host_list_poll(&self, host: &str, now: Instant) -> bool {
        let inner = self.inner.read().await;
        inner
            .list_backoff_until
            .get(host)
            .map(|until| now < *until)
            .unwrap_or(false)
    }

    pub async fn note_host_list_timeout(&self, host: &str, now: Instant, backoff: Duration) {
        self.inner
            .write()
            .await
            .list_backoff_until
            .insert(host.to_string(), now + backoff);
    }

    pub async fn clear_host_list_backoff(&self, host: &str) {
        self.inner.write().await.list_backoff_until.remove(host);
    }

    pub async fn resolve_agent_for_task(&self, task_id: TaskId) -> Option<ConnectedAgent> {
        let host = self.get_host_for_task(task_id).await?;
        self.get_agent_for_host(&host).await
    }

    pub async fn subscribe_to_task(&self, id: TaskId) -> broadcast::Receiver<AgentEvent> {
        let mut inner = self.inner.write().await;
        let tx = inner
            .event_channels
            .entry(id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(200);
                tx
            })
            .clone();
        tx.subscribe()
    }

    pub async fn broadcast_task_event(&self, task_id: TaskId, event: AgentEvent) {
        let mut inner = self.inner.write().await;
        let tx = inner
            .event_channels
            .entry(task_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(200);
                tx
            })
            .clone();
        let _ = tx.send(event);
    }

    pub async fn subscribe_to_terminal(
        &self,
        terminal_id: Uuid,
    ) -> broadcast::Receiver<TerminalEvent> {
        let mut inner = self.inner.write().await;
        let tx = inner
            .terminal_channels
            .entry(terminal_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(200);
                tx
            })
            .clone();
        tx.subscribe()
    }

    pub async fn broadcast_terminal_event(&self, terminal_id: Uuid, event: TerminalEvent) {
        let mut inner = self.inner.write().await;
        let tx = inner
            .terminal_channels
            .entry(terminal_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(200);
                tx
            })
            .clone();
        let _ = tx.send(event.clone());
        if matches!(event, TerminalEvent::Closed | TerminalEvent::Error(_)) {
            inner.terminal_channels.remove(&terminal_id);
        }
    }
}

fn unique_host_label(base: &str, existing: &HashMap<String, Uuid>) -> String {
    if !existing.contains_key(base) {
        return base.to_string();
    }
    for idx in 2..1000 {
        let candidate = format!("{}-{}", base, idx);
        if !existing.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{}-{}", base, Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use slopcoder_core::{
        anyagent::AgentKind,
        environment::Environment,
        task::{Task, TaskWorkspaceKind},
    };
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn host_list_timeout_backoff_expires() {
        let state = AppState::new(None, "test-password".to_string());
        let host = "boa";
        let now = Instant::now();

        assert!(!state.should_skip_host_list_poll(host, now).await);
        state
            .note_host_list_timeout(host, now, Duration::from_secs(5))
            .await;
        assert!(
            state
                .should_skip_host_list_poll(host, now + Duration::from_secs(4))
                .await
        );
        assert!(
            !state
                .should_skip_host_list_poll(host, now + Duration::from_secs(6))
                .await
        );
    }

    #[tokio::test]
    async fn caches_environment_snapshots_per_host() {
        let state = AppState::new(None, "test-password".to_string());
        let host = "boa";
        let environments = vec![
            Environment {
                name: "alpha".to_string(),
                directory: PathBuf::from("/tmp/alpha"),
            },
            Environment {
                name: "beta".to_string(),
                directory: PathBuf::from("/tmp/beta"),
            },
        ];

        state.cache_environments_for_host(host, &environments).await;
        assert_eq!(
            state.get_cached_environments_for_host(host).await.len(),
            environments.len()
        );
    }

    #[tokio::test]
    async fn caches_task_snapshots_per_host() {
        let state = AppState::new(None, "test-password".to_string());
        let host = "boa";
        let tasks = vec![Task::new(
            AgentKind::default(),
            "environment".to_string(),
            "task".to_string(),
            TaskWorkspaceKind::Environment,
            None,
            None,
            false,
            PathBuf::from("/tmp/workspace"),
        )];

        state.cache_tasks_for_host(host, &tasks).await;
        assert_eq!(
            state.get_cached_tasks_for_host(host).await.len(),
            tasks.len()
        );
    }
}
