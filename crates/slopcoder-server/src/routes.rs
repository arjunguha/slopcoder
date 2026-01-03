//! HTTP routes for the Slopcoder API.

use crate::state::{AppState, StateError};
use serde::{Deserialize, Serialize};
use slopcoder_core::{
    agent::Agent,
    task::{Task, TaskId},
};
use std::convert::Infallible;
use uuid::Uuid;
use warp::{http::StatusCode, Filter, Reply};

/// Create all API routes.
pub fn routes(
    state: AppState,
) -> impl Filter<Extract = (impl Reply,), Error = warp::Rejection> + Clone {
    let api = warp::path("api");

    let environments = api
        .and(warp::path("environments"))
        .and(environments_routes(state.clone()));

    let tasks = api
        .and(warp::path("tasks"))
        .and(tasks_routes(state.clone()));

    environments.or(tasks)
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

    let branches = warp::path!(String / "branches")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(list_branches);

    list.or(branches)
}

#[derive(Serialize)]
struct EnvironmentResponse {
    name: String,
    directory: String,
}

async fn list_environments(state: AppState) -> Result<impl Reply, Infallible> {
    let config = state.get_config().await;
    let environments: Vec<EnvironmentResponse> = config
        .environments
        .iter()
        .map(|e| EnvironmentResponse {
            name: e.name.clone(),
            directory: e.directory.to_string_lossy().to_string(),
        })
        .collect();

    Ok(warp::reply::json(&environments))
}

#[derive(Serialize)]
struct BranchesResponse {
    branches: Vec<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn list_branches(name: String, state: AppState) -> Result<impl Reply, Infallible> {
    let config = state.get_config().await;

    let Some(env) = config.find(&name) else {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: format!("Environment '{}' not found", name),
            }),
            StatusCode::NOT_FOUND,
        ));
    };

    match env.list_branches().await {
        Ok(branches) => Ok(warp::reply::with_status(
            warp::reply::json(&BranchesResponse { branches }),
            StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: e.to_string(),
            }),
            StatusCode::INTERNAL_SERVER_ERROR,
        )),
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

    let stream = warp::path!(String / "stream")
        .and(warp::ws())
        .and(with_state(state.clone()))
        .map(|id: String, ws: warp::ws::Ws, state: AppState| {
            ws.on_upgrade(move |socket| handle_websocket(socket, id, state))
        });

    list.or(create).or(get).or(prompt).or(stream)
}

#[derive(Serialize)]
struct TaskResponse {
    id: String,
    name: String,
    environment: String,
    branch: String,
    status: String,
    session_id: Option<String>,
    created_at: String,
    history: Vec<PromptRunResponse>,
}

#[derive(Serialize)]
struct PromptRunResponse {
    prompt: String,
    started_at: String,
    finished_at: Option<String>,
    success: Option<bool>,
}

impl From<&Task> for TaskResponse {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            name: task.name.clone(),
            environment: task.environment.clone(),
            branch: task.branch.clone(),
            status: format!("{:?}", task.status).to_lowercase(),
            session_id: task.session_id.map(|id| id.to_string()),
            created_at: task.created_at.to_rfc3339(),
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
    let tasks = state.list_tasks().await;
    let responses: Vec<TaskResponse> = tasks.iter().map(TaskResponse::from).collect();
    Ok(warp::reply::json(&responses))
}

async fn get_task(id: String, state: AppState) -> Result<impl Reply, Infallible> {
    let Ok(uuid) = Uuid::parse_str(&id) else {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: "Invalid task ID".to_string(),
            }),
            StatusCode::BAD_REQUEST,
        ));
    };

    let task_id = TaskId(uuid);

    match state.get_task(task_id).await {
        Some(task) => Ok(warp::reply::with_status(
            warp::reply::json(&TaskResponse::from(&task)),
            StatusCode::OK,
        )),
        None => Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: "Task not found".to_string(),
            }),
            StatusCode::NOT_FOUND,
        )),
    }
}

#[derive(Deserialize)]
struct CreateTaskRequest {
    name: String,
    environment: String,
    branch: String,
    prompt: String,
}

#[derive(Serialize)]
struct CreateTaskResponse {
    id: String,
    worktree_path: String,
}

async fn create_task(
    req: CreateTaskRequest,
    state: AppState,
) -> Result<impl Reply, Infallible> {
    let config = state.get_config().await;

    // Find the environment
    let Some(env) = config.find(&req.environment) else {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: format!("Environment '{}' not found", req.environment),
            }),
            StatusCode::NOT_FOUND,
        ));
    };

    // Create or get worktree
    let worktree_path = if env.worktree_exists(&req.branch) {
        env.worktree_path(&req.branch)
    } else {
        match env.create_worktree(&req.branch).await {
            Ok(path) => path,
            Err(e) => {
                return Ok(warp::reply::with_status(
                    warp::reply::json(&ErrorResponse {
                        error: format!("Failed to create worktree: {}", e),
                    }),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ));
            }
        }
    };

    // Create the task
    let task = Task::new(
        req.name,
        req.environment,
        req.branch,
        worktree_path.clone(),
    );

    let task_id = task.id;
    let response = CreateTaskResponse {
        id: task_id.to_string(),
        worktree_path: worktree_path.to_string_lossy().to_string(),
    };

    if let Err(e) = state.insert_task(task).await {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: format!("Failed to save task: {}", e),
            }),
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }

    // Start the agent
    let state_clone = state.clone();
    let prompt = req.prompt.clone();
    tokio::spawn(async move {
        run_agent(state_clone, task_id, prompt, None).await;
    });

    Ok(warp::reply::with_status(
        warp::reply::json(&response),
        StatusCode::CREATED,
    ))
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
    let Ok(uuid) = Uuid::parse_str(&id) else {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: "Invalid task ID".to_string(),
            }),
            StatusCode::BAD_REQUEST,
        ));
    };

    let task_id = TaskId(uuid);

    let task = match state.get_task(task_id).await {
        Some(t) => t,
        None => {
            return Ok(warp::reply::with_status(
                warp::reply::json(&ErrorResponse {
                    error: "Task not found".to_string(),
                }),
                StatusCode::NOT_FOUND,
            ));
        }
    };

    if !task.can_run() {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: "Task is currently running".to_string(),
            }),
            StatusCode::CONFLICT,
        ));
    }

    // Check worktree still exists
    if !state.validate_task_worktree(task_id).await {
        return Ok(warp::reply::with_status(
            warp::reply::json(&ErrorResponse {
                error: "Task worktree no longer exists (may have been removed from CLI)".to_string(),
            }),
            StatusCode::GONE,
        ));
    }

    let session_id = task.session_id;

    // Start the agent
    let state_clone = state.clone();
    let prompt = req.prompt.clone();
    tokio::spawn(async move {
        run_agent(state_clone, task_id, prompt, session_id).await;
    });

    Ok(warp::reply::with_status(
        warp::reply::json(&serde_json::json!({"status": "started"})),
        StatusCode::OK,
    ))
}

/// Run the agent for a task.
async fn run_agent(state: AppState, task_id: TaskId, prompt: String, session_id: Option<Uuid>) {
    let task = match state.get_task(task_id).await {
        Some(t) => t,
        None => {
            tracing::error!("Task {} not found", task_id);
            return;
        }
    };

    // Mark the task as running
    if let Err(e) = state.start_task_run(task_id, prompt.clone()).await {
        match e {
            StateError::WorktreeMissing(_) => {
                tracing::error!("Task {} worktree no longer exists", task_id);
            }
            _ => {
                tracing::error!("Failed to start task run for {}: {}", task_id, e);
            }
        }
        return;
    }

    // Create event channel for this task
    let _event_tx = state.create_event_channel(task_id).await;

    let agent_config = state.get_agent_config().await;

    // Spawn the agent
    let agent_result = if let Some(sid) = session_id {
        Agent::resume(&agent_config, &task.worktree_path, sid, &prompt).await
    } else {
        Agent::spawn(&agent_config, &task.worktree_path, &prompt).await
    };

    let mut agent = match agent_result {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to spawn agent: {}", e);
            let _ = state.complete_task_run(task_id, false).await;
            return;
        }
    };

    // Stream events
    while let Some(result) = agent.next_event().await {
        match result {
            Ok(event) => {
                // Update session ID if we got it
                if let Some(sid) = event.session_id() {
                    if let Err(e) = state.set_task_session_id(task_id, sid).await {
                        tracing::warn!("Failed to save session ID: {}", e);
                    }
                }
                // Broadcast to WebSocket clients
                state.broadcast_event(task_id, event).await;
            }
            Err(e) => {
                tracing::warn!("Error reading event: {}", e);
            }
        }
    }

    // Wait for completion
    let result = agent.wait().await;
    let success = match &result {
        Ok(r) => {
            // Update session ID from result
            if let Err(e) = state.set_task_session_id(task_id, r.session_id).await {
                tracing::warn!("Failed to save session ID: {}", e);
            }
            r.success
        }
        Err(_) => false,
    };

    if let Err(e) = state.complete_task_run(task_id, success).await {
        tracing::warn!("Failed to save task completion: {}", e);
    }
    tracing::info!("Task {} completed with success={}", task_id, success);
}

// ============================================================================
// WebSocket handler
// ============================================================================

use futures::{SinkExt, StreamExt};
use warp::ws::{Message, WebSocket};

async fn handle_websocket(ws: WebSocket, id: String, state: AppState) {
    let Ok(uuid) = Uuid::parse_str(&id) else {
        tracing::warn!("Invalid task ID in WebSocket connection: {}", id);
        return;
    };

    let task_id = TaskId(uuid);

    // Subscribe to events
    let mut rx = match state.subscribe_to_task(task_id).await {
        Some(rx) => rx,
        None => {
            tracing::warn!("No event channel for task {}", task_id);
            return;
        }
    };

    let (mut tx, mut _rx) = ws.split();

    // Forward events to WebSocket
    while let Ok(event) = rx.recv().await {
        let json = match serde_json::to_string(&event) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize event: {}", e);
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

fn with_state(
    state: AppState,
) -> impl Filter<Extract = (AppState,), Error = Infallible> + Clone {
    warp::any().map(move || state.clone())
}
