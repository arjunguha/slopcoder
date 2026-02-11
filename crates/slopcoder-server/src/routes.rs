//! HTTP routes for the Slopcoder coordinator API.

use crate::state::{AppState, ConnectedAgent, RemoteError, StateError};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use slopcoder_core::{
    agent_rpc::{AgentCreateTaskRequest, AgentEnvelope, AgentRequest, AgentResponse},
    task::{Task, TaskId},
    AgentEvent,
};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;
use warp::http::{Method, StatusCode};
use warp::reject::InvalidQuery;
use warp::ws::{Message, WebSocket};
use warp::{Filter, Reply};

#[derive(Debug)]
struct AuthError;
impl warp::reject::Reject for AuthError {}

/// Create all API routes.
pub fn routes(
    state: AppState,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    let hosts = warp::path("hosts").and(hosts_routes(state.clone()));
    let environments = warp::path("environments").and(environments_routes(state.clone()));
    let tasks = warp::path("tasks").and(tasks_routes(state.clone()));

    let api_scoped = auth_filter_api(state.clone())
        .and(hosts.or(environments).or(tasks))
        .recover(handle_rejection);
    let api_routes = warp::path("api").and(api_scoped);

    let agent_connect = warp::path!("agent" / "connect")
        .and(auth_filter_agent(state.clone()))
        .and(warp::ws())
        .and(with_state(state))
        .map(|ws: warp::ws::Ws, state: AppState| {
            ws.on_upgrade(move |socket| handle_agent_socket(socket, state))
        });

    api_routes.or(agent_connect)
}

// ============================================================================
// Hosts
// ============================================================================

fn hosts_routes(
    state: AppState,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    warp::path::end()
        .and(warp::get())
        .and(with_state(state))
        .and_then(list_hosts)
}

#[derive(Serialize)]
struct HostResponse {
    host: String,
    hostname: String,
    connected_at: String,
}

async fn list_hosts(state: AppState) -> Result<impl Reply, Infallible> {
    let hosts = state.list_hosts().await;
    let response: Vec<HostResponse> = hosts
        .into_iter()
        .map(|h| HostResponse {
            host: h.host,
            hostname: h.hostname,
            connected_at: h.connected_at.to_rfc3339(),
        })
        .collect();
    Ok(warp::reply::json(&response))
}

// ============================================================================
// Environment routes
// ============================================================================

fn environments_routes(
    state: AppState,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    let list = warp::path::end()
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(list_environments);

    let create = warp::path::end()
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(create_environment);

    let branches = warp::path!(String / "branches")
        .and(warp::get())
        .and(warp::query::<HostQuery>())
        .and(with_state(state))
        .and_then(list_branches);

    list.or(create).or(branches)
}

#[derive(Serialize)]
struct EnvironmentResponse {
    host: String,
    name: String,
    directory: String,
}

async fn list_environments(state: AppState) -> Result<impl Reply, Infallible> {
    let agents = state.list_agents().await;
    let mut environments = Vec::new();

    for agent in agents {
        match agent.request(AgentRequest::ListEnvironments).await {
            Ok(AgentResponse::Environments { environments: envs }) => {
                for env in envs {
                    environments.push(EnvironmentResponse {
                        host: agent.host.clone(),
                        name: env.name,
                        directory: env.directory.to_string_lossy().to_string(),
                    });
                }
            }
            Ok(_) => {
                tracing::warn!(
                    "Unexpected response for list environments from {}",
                    agent.host
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to list environments from host '{}': {}",
                    agent.host,
                    e
                );
            }
        }
    }

    environments.sort_by(|a, b| {
        (a.host.as_str(), a.name.as_str()).cmp(&(b.host.as_str(), b.name.as_str()))
    });
    Ok(warp::reply::json(&environments))
}

#[derive(Deserialize)]
struct CreateEnvironmentRequest {
    host: String,
    name: String,
}

async fn create_environment(
    req: CreateEnvironmentRequest,
    state: AppState,
) -> Result<impl Reply, Infallible> {
    let host = req.host.trim();
    if host.is_empty() {
        return Ok(error_reply(StatusCode::BAD_REQUEST, "Host is required"));
    }

    let agent = match state.get_agent_for_host(host).await {
        Some(agent) => agent,
        None => {
            return Ok(error_reply(
                StatusCode::NOT_FOUND,
                format!("Host '{}' is not connected", host),
            ));
        }
    };

    match agent
        .request(AgentRequest::CreateEnvironment {
            name: req.name.clone(),
        })
        .await
    {
        Ok(AgentResponse::Environment { environment }) => Ok(warp::reply::with_status(
            warp::reply::json(&EnvironmentResponse {
                host: agent.host,
                name: environment.name,
                directory: environment.directory.to_string_lossy().to_string(),
            }),
            StatusCode::CREATED,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

#[derive(Deserialize)]
struct HostQuery {
    host: Option<String>,
}

#[derive(Serialize)]
struct BranchesResponse {
    branches: Vec<String>,
}

async fn list_branches(
    name: String,
    query: HostQuery,
    state: AppState,
) -> Result<impl Reply, Infallible> {
    let decoded_name = match urlencoding::decode(&name) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => {
            return Ok(error_reply(
                StatusCode::BAD_REQUEST,
                "Environment name must be valid URL encoding",
            ));
        }
    };

    let agent = match pick_agent(state.clone(), query.host.as_deref()).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent
        .request(AgentRequest::ListBranches {
            environment: decoded_name,
        })
        .await
    {
        Ok(AgentResponse::Branches { branches }) => Ok(warp::reply::with_status(
            warp::reply::json(&BranchesResponse { branches }),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

// ============================================================================
// Task routes
// ============================================================================

fn tasks_routes(
    state: AppState,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    let list = warp::path::end()
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(list_tasks);

    let create = warp::path::end()
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(create_task);

    let get = warp::path!(String)
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(get_task);

    let prompt = warp::path!(String / "prompt")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(send_prompt);

    let output = warp::path!(String / "output")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(get_task_output);

    let diff = warp::path!(String / "diff")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(get_task_diff);

    let interrupt = warp::path!(String / "interrupt")
        .and(warp::post())
        .and(with_state(state.clone()))
        .and_then(interrupt_task);

    let stream = warp::path!(String / "stream")
        .and(warp::ws())
        .and(with_state(state.clone()))
        .map(|id: String, ws: warp::ws::Ws, state: AppState| {
            ws.on_upgrade(move |socket| handle_task_websocket(socket, id, state))
        });

    let merge = warp::path!(String / "merge")
        .and(warp::post())
        .and(with_state(state))
        .and_then(merge_task);

    list.or(create)
        .or(get)
        .or(prompt)
        .or(output)
        .or(diff)
        .or(interrupt)
        .or(stream)
        .or(merge)
}

#[derive(Serialize)]
struct TaskResponse {
    id: String,
    host: String,
    agent: String,
    environment: String,
    name: String,
    workspace_kind: String,
    base_branch: Option<String>,
    merge_branch: Option<String>,
    status: String,
    session_id: Option<String>,
    created_at: String,
    worktree_date: Option<String>,
    history: Vec<PromptRunResponse>,
}

#[derive(Serialize)]
struct PromptRunResponse {
    prompt: String,
    started_at: String,
    finished_at: Option<String>,
    success: Option<bool>,
}

impl TaskResponse {
    fn from_task(host: &str, task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            host: host.to_string(),
            agent: format!("{:?}", task.agent).to_lowercase(),
            environment: task.environment.clone(),
            name: task.name.clone(),
            workspace_kind: format!("{:?}", task.workspace_kind).to_lowercase(),
            base_branch: task.base_branch.clone(),
            merge_branch: task.merge_branch.clone(),
            status: format!("{:?}", task.status).to_lowercase(),
            session_id: task.session_id.map(|id| id.to_string()),
            created_at: task.created_at.to_rfc3339(),
            worktree_date: None,
            history: task
                .history
                .iter()
                .map(|r| PromptRunResponse {
                    prompt: r.prompt.clone(),
                    started_at: r.started_at.to_rfc3339(),
                    finished_at: r.finished_at.map(|t| t.to_rfc3339()),
                    success: r.success,
                })
                .collect(),
        }
    }
}

async fn list_tasks(state: AppState) -> Result<impl Reply, Infallible> {
    let agents = state.list_agents().await;
    let mut tasks = Vec::new();

    for agent in agents {
        match agent.request(AgentRequest::ListTasks).await {
            Ok(AgentResponse::Tasks { tasks: host_tasks }) => {
                state.record_tasks_for_host(&agent.host, &host_tasks).await;
                tasks.extend(
                    host_tasks
                        .iter()
                        .map(|task| TaskResponse::from_task(&agent.host, task)),
                );
            }
            Ok(_) => tracing::warn!("Unexpected list_tasks response from {}", agent.host),
            Err(e) => tracing::warn!("Failed to list tasks from '{}': {}", agent.host, e),
        }
    }

    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(warp::reply::json(&tasks))
}

async fn get_task(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    match find_task(&state, task_id).await {
        Ok(Some((host, task))) => Ok(warp::reply::with_status(
            warp::reply::json(&TaskResponse::from_task(&host, &task)),
            StatusCode::OK,
        )),
        Ok(None) => Ok(error_reply(StatusCode::NOT_FOUND, "Task not found")),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

#[derive(Deserialize)]
struct CreateTaskRequest {
    host: String,
    environment: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    use_worktree: bool,
    prompt: String,
    #[serde(default)]
    agent: Option<slopcoder_core::anyagent::AgentKind>,
}

#[derive(Serialize)]
struct CreateTaskResponse {
    id: String,
    worktree_path: String,
}

async fn create_task(req: CreateTaskRequest, state: AppState) -> Result<impl Reply, Infallible> {
    let host = req.host.trim();
    if host.is_empty() {
        return Ok(error_reply(StatusCode::BAD_REQUEST, "Host is required"));
    }
    let agent = match state.get_agent_for_host(host).await {
        Some(agent) => agent,
        None => {
            return Ok(error_reply(
                StatusCode::NOT_FOUND,
                format!("Host '{}' is not connected", host),
            ));
        }
    };

    let request = AgentCreateTaskRequest {
        environment: req.environment,
        name: req.name,
        use_worktree: req.use_worktree,
        prompt: req.prompt,
        agent: req.agent,
    };

    match agent.request(AgentRequest::CreateTask { request }).await {
        Ok(AgentResponse::CreatedTask { id, worktree_path }) => {
            state.set_task_host(id, agent.host).await;
            Ok(warp::reply::with_status(
                warp::reply::json(&CreateTaskResponse {
                    id: id.to_string(),
                    worktree_path,
                }),
                StatusCode::CREATED,
            ))
        }
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

#[derive(Deserialize)]
struct SendPromptRequest {
    prompt: String,
}

async fn send_prompt(
    id: String,
    req: SendPromptRequest,
    state: AppState,
) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    let agent = match resolve_agent_for_task(&state, task_id).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent
        .request(AgentRequest::SendPrompt {
            task_id,
            prompt: req.prompt,
        })
        .await
    {
        Ok(AgentResponse::Ack) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "status": "started" })),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

#[derive(Serialize)]
struct TaskOutputResponse {
    events: Vec<AgentEvent>,
}

async fn get_task_output(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    let agent = match resolve_agent_for_task(&state, task_id).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent.request(AgentRequest::GetTaskOutput { task_id }).await {
        Ok(AgentResponse::TaskOutput { events }) => Ok(warp::reply::with_status(
            warp::reply::json(&TaskOutputResponse { events }),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

#[derive(Serialize)]
struct TaskDiffResponse {
    staged: String,
    unstaged: String,
}

async fn get_task_diff(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    let agent = match resolve_agent_for_task(&state, task_id).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent.request(AgentRequest::GetTaskDiff { task_id }).await {
        Ok(AgentResponse::TaskDiff { staged, unstaged }) => Ok(warp::reply::with_status(
            warp::reply::json(&TaskDiffResponse { staged, unstaged }),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

async fn interrupt_task(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    let agent = match resolve_agent_for_task(&state, task_id).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent.request(AgentRequest::InterruptTask { task_id }).await {
        Ok(AgentResponse::Ack) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "status": "interrupted" })),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

async fn merge_task(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let task_id = match parse_task_id(&id) {
        Ok(id) => id,
        Err(reply) => return Ok(reply),
    };

    let agent = match resolve_agent_for_task(&state, task_id).await {
        Ok(agent) => agent,
        Err(e) => return Ok(error_reply(state_error_status(&e), e.to_string())),
    };

    match agent.request(AgentRequest::MergeTask { task_id }).await {
        Ok(AgentResponse::MergeResult { status, message }) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "status": status, "message": message })),
            StatusCode::OK,
        )),
        Ok(_) => Ok(error_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response from agent",
        )),
        Err(e) => Ok(error_reply(state_error_status(&e), e.to_string())),
    }
}

async fn resolve_agent_for_task(
    state: &AppState,
    task_id: TaskId,
) -> Result<ConnectedAgent, StateError> {
    if let Some(agent) = state.resolve_agent_for_task(task_id).await {
        return Ok(agent);
    }

    let agents = state.list_agents().await;
    for agent in agents {
        let response = agent.request(AgentRequest::GetTask { task_id }).await;
        if let Ok(AgentResponse::Task { task: Some(_) }) = response {
            state.set_task_host(task_id, agent.host.clone()).await;
            return Ok(agent);
        }
    }

    Err(StateError::RemoteError {
        status: StatusCode::NOT_FOUND.as_u16(),
        error: "Task not found".to_string(),
    })
}

async fn find_task(
    state: &AppState,
    task_id: TaskId,
) -> Result<Option<(String, Task)>, StateError> {
    if let Some(agent) = state.resolve_agent_for_task(task_id).await {
        match agent.request(AgentRequest::GetTask { task_id }).await {
            Ok(AgentResponse::Task { task: Some(task) }) => {
                return Ok(Some((agent.host, task)));
            }
            Ok(AgentResponse::Task { task: None }) => {
                state.clear_task_host(task_id).await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to fetch mapped task {}: {}", task_id, e);
            }
        }
    }

    let agents = state.list_agents().await;
    for agent in agents {
        match agent.request(AgentRequest::GetTask { task_id }).await {
            Ok(AgentResponse::Task { task: Some(task) }) => {
                state.set_task_host(task_id, agent.host.clone()).await;
                return Ok(Some((agent.host, task)));
            }
            Ok(AgentResponse::Task { task: None }) => {}
            Ok(_) => {}
            Err(e) => tracing::warn!("Failed to query task {} on {}: {}", task_id, agent.host, e),
        }
    }
    Ok(None)
}

// ============================================================================
// Agent websocket
// ============================================================================

async fn handle_agent_socket(ws: WebSocket, state: AppState) {
    let (mut sink, mut stream) = ws.split();

    let hello = match stream.next().await {
        Some(Ok(msg)) if msg.is_text() => match msg.to_str() {
            Ok(text) => match serde_json::from_str::<AgentEnvelope>(text) {
                Ok(AgentEnvelope::Hello {
                    hostname,
                    display_name,
                }) => (hostname, display_name),
                _ => {
                    let _ = sink.send(Message::text("expected hello")).await;
                    return;
                }
            },
            Err(_) => return,
        },
        _ => return,
    };

    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<AgentEnvelope>();
    let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<AgentResponse, RemoteError>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let agent = state
        .register_agent(
            hello.0.clone(),
            hello.1.clone(),
            outbound_tx.clone(),
            pending.clone(),
        )
        .await;
    tracing::info!(
        "Agent connected host='{}' hostname='{}'",
        agent.host,
        agent.hostname
    );

    let writer = tokio::spawn(async move {
        while let Some(envelope) = outbound_rx.recv().await {
            let payload = match serde_json::to_string(&envelope) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Failed to serialize envelope for agent write: {}", e);
                    continue;
                }
            };
            if sink.send(Message::text(payload)).await.is_err() {
                break;
            }
        }
    });

    while let Some(incoming) = stream.next().await {
        let Ok(message) = incoming else {
            break;
        };
        if message.is_close() {
            break;
        }
        if !message.is_text() {
            continue;
        }

        let text = match message.to_str() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let envelope = match serde_json::from_str::<AgentEnvelope>(text) {
            Ok(env) => env,
            Err(e) => {
                tracing::warn!("Failed to decode agent envelope from {}: {}", agent.host, e);
                continue;
            }
        };

        match envelope {
            AgentEnvelope::Response {
                request_id,
                response,
            } => {
                if let Some(tx) = pending.lock().await.remove(&request_id) {
                    let _ = tx.send(Ok(response));
                }
            }
            AgentEnvelope::Error {
                request_id,
                status,
                error,
            } => {
                if let Some(tx) = pending.lock().await.remove(&request_id) {
                    let _ = tx.send(Err(RemoteError { status, error }));
                }
            }
            AgentEnvelope::TaskEvent { task_id, event } => {
                state.set_task_host(task_id, agent.host.clone()).await;
                state.broadcast_task_event(task_id, event).await;
            }
            AgentEnvelope::Hello { .. } | AgentEnvelope::Request { .. } => {
                tracing::warn!("Ignoring unexpected envelope from agent '{}'", agent.host);
            }
        }
    }

    let mut pending_locked = pending.lock().await;
    for (_, tx) in pending_locked.drain() {
        let _ = tx.send(Err(RemoteError {
            status: StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            error: "Agent disconnected".to_string(),
        }));
    }
    drop(pending_locked);

    writer.abort();
    state.unregister_agent(agent.id).await;
}

// ============================================================================
// Task event websocket for UI
// ============================================================================

async fn handle_task_websocket(ws: WebSocket, id: String, state: AppState) {
    let Ok(uuid) = Uuid::parse_str(&id) else {
        tracing::warn!("Invalid task ID in websocket: {}", id);
        return;
    };
    let task_id = TaskId(uuid);
    let mut rx = state.subscribe_to_task(task_id).await;
    let (mut tx, mut _rx) = ws.split();

    while let Ok(event) = rx.recv().await {
        let json = match serde_json::to_string(&event) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize task event: {}", e);
                continue;
            }
        };
        if tx.send(Message::text(json)).await.is_err() {
            break;
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn error_reply(
    status: StatusCode,
    error: impl Into<String>,
) -> warp::reply::WithStatus<warp::reply::Json> {
    warp::reply::with_status(
        warp::reply::json(&ErrorResponse {
            error: error.into(),
        }),
        status,
    )
}

fn parse_task_id(id: &str) -> Result<TaskId, warp::reply::WithStatus<warp::reply::Json>> {
    Uuid::parse_str(id)
        .map(TaskId)
        .map_err(|_| error_reply(StatusCode::BAD_REQUEST, "Invalid task ID"))
}

fn with_state(state: AppState) -> impl Filter<Extract = (AppState,), Error = Infallible> + Clone {
    warp::any().map(move || state.clone())
}

fn auth_filter_api(state: AppState) -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    let raw_query = warp::query::raw()
        .or(warp::any().map(|| "".to_string()))
        .unify();

    warp::any()
        .and(with_state(state))
        .and(warp::method())
        .and(warp::header::optional::<String>("x-slopcoder-password"))
        .and(raw_query)
        .and_then(check_api_auth)
        .untuple_one()
}

fn auth_filter_agent(
    state: AppState,
) -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    let raw_query = warp::query::raw()
        .or(warp::any().map(|| "".to_string()))
        .unify();

    warp::any()
        .and(with_state(state))
        .and(warp::method())
        .and(warp::header::optional::<String>("x-slopcoder-password"))
        .and(raw_query)
        .and_then(check_agent_auth)
        .untuple_one()
}

async fn check_api_auth(
    state: AppState,
    method: Method,
    header_password: Option<String>,
    raw_query: String,
) -> Result<(), warp::Rejection> {
    if method == Method::OPTIONS {
        return Ok(());
    }
    let required = state.get_ui_auth_password().await;
    if let Some(required) = required {
        let query_password = extract_password_from_query(&raw_query);
        let provided = header_password.or(query_password);
        if provided.as_deref() != Some(required.as_str()) {
            return Err(warp::reject::custom(AuthError));
        }
    }
    Ok(())
}

async fn check_agent_auth(
    state: AppState,
    method: Method,
    header_password: Option<String>,
    raw_query: String,
) -> Result<(), warp::Rejection> {
    if method == Method::OPTIONS {
        return Ok(());
    }
    let required = state.get_agent_auth_password().await;
    let query_password = extract_password_from_query(&raw_query);
    let provided = header_password.or(query_password);
    if provided.as_deref() != Some(required.as_str()) {
        return Err(warp::reject::custom(AuthError));
    }
    Ok(())
}

fn extract_password_from_query(raw_query: &str) -> Option<String> {
    if raw_query.is_empty() {
        return None;
    }

    for pair in raw_query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let decoded_key = urlencoding::decode(key).ok()?;
        if decoded_key == "password" {
            let value = parts.next().unwrap_or("");
            return urlencoding::decode(value).ok().map(|v| v.into_owned());
        }
    }
    None
}

async fn handle_rejection(err: warp::Rejection) -> Result<impl Reply, Infallible> {
    if err.find::<AuthError>().is_some() {
        return Ok(error_reply(StatusCode::UNAUTHORIZED, "Unauthorized"));
    }
    if err.is_not_found() {
        return Ok(error_reply(StatusCode::NOT_FOUND, "Not Found"));
    }
    if err.find::<InvalidQuery>().is_some() {
        return Ok(error_reply(StatusCode::BAD_REQUEST, "Invalid query"));
    }
    tracing::error!("Unhandled API rejection: {:?}", err);
    Ok(error_reply(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal Server Error",
    ))
}

async fn pick_agent(state: AppState, host: Option<&str>) -> Result<ConnectedAgent, StateError> {
    if let Some(host) = host {
        return state
            .get_agent_for_host(host)
            .await
            .ok_or_else(|| StateError::HostNotConnected(host.to_string()));
    }

    let agents = state.list_agents().await;
    match agents.len() {
        0 => Err(StateError::NoAgentsConnected),
        1 => Ok(agents[0].clone()),
        _ => Err(StateError::HostRequired),
    }
}

fn state_error_status(err: &StateError) -> StatusCode {
    match err {
        StateError::HostRequired => StatusCode::BAD_REQUEST,
        StateError::HostNotConnected(_) => StatusCode::NOT_FOUND,
        StateError::NoAgentsConnected => StatusCode::SERVICE_UNAVAILABLE,
        StateError::AgentDisconnected | StateError::AgentTimeout => StatusCode::SERVICE_UNAVAILABLE,
        StateError::RemoteError { status, .. } => {
            StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_password_from_query;

    #[test]
    fn test_extract_password() {
        assert_eq!(
            extract_password_from_query("foo=bar&password=abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_password_from_query("password=hello%20world"),
            Some("hello world".to_string())
        );
        assert_eq!(extract_password_from_query("foo=bar"), None);
        assert_eq!(extract_password_from_query(""), None);
    }
}
