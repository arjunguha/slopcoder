mod state;

use futures::{SinkExt, StreamExt};
use http::StatusCode;
use slopcoder_core::{
    agent_rpc::{AgentCreateTaskRequest, AgentEnvelope, AgentRequest, AgentResponse},
    anyagent::{resume_anyagent, spawn_anyagent},
    branch_picker::pick_feature_branch,
    task::{Task, TaskId},
    AgentEvent,
};
use state::{AppState, StateError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, client::IntoClientRequest, Message},
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;

#[derive(Debug)]
struct RpcError {
    status: u16,
    error: String,
}

impl RpcError {
    fn new(status: StatusCode, error: impl Into<String>) -> Self {
        Self {
            status: status.as_u16(),
            error: error.into(),
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("slopagent=info".parse().unwrap()))
        .init();

    let mut args = std::env::args().skip(1);
    let mut config_path: Option<PathBuf> = None;
    let mut server_url: Option<String> = None;
    let mut branch_model = "claude-haiku-4-5".to_string();
    let mut provided_password: Option<String> = None;
    let mut host_override: Option<String> = None;
    let mut no_password = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" | "--coordinator" => server_url = args.next(),
            "--branch-model" => {
                if let Some(value) = args.next() {
                    branch_model = value;
                }
            }
            "--password" => provided_password = args.next(),
            "--name" | "--hostname" => host_override = args.next(),
            "--no-password" => no_password = true,
            "-h" | "--help" => {
                println!(
                    "Usage: slopagent [config.yaml] --server ws://HOST:PORT [--name HOSTNAME] [--password VALUE|--no-password] [--branch-model MODEL]\n\
Defaults: config=environments.yaml, branch-model=claude-haiku-4-5"
                );
                return;
            }
            _ => {
                if config_path.is_none() {
                    config_path = Some(PathBuf::from(arg));
                }
            }
        }
    }

    let config_path = config_path.unwrap_or_else(|| PathBuf::from("environments.yaml"));
    if !config_path.exists() {
        tracing::error!("Config file not found: {}", config_path.display());
        std::process::exit(1);
    }

    let server_url = match server_url {
        Some(url) => normalize_server_url(&url),
        None => {
            tracing::error!("Missing --server argument");
            std::process::exit(1);
        }
    };

    let password = if no_password {
        None
    } else if let Some(password) = provided_password {
        Some(password)
    } else {
        prompt_password()
    };

    let state = match AppState::new(config_path.clone(), branch_model).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Startup checks failed: {}", e);
            std::process::exit(1);
        }
    };

    let hostname = default_hostname();
    if let Some(display_name) = host_override.as_deref() {
        tracing::info!(
            "slopagent hostname: {} (display override: {})",
            hostname,
            display_name
        );
    } else {
        tracing::info!("slopagent hostname: {}", hostname);
    }

    loop {
        match run_connection(
            state.clone(),
            &server_url,
            password.clone(),
            hostname.clone(),
            host_override.clone(),
        )
        .await
        {
            Ok(()) => {
                tracing::warn!("Disconnected from coordinator; retrying in 2s");
            }
            Err(e) => {
                tracing::warn!("Connection error: {}; retrying in 2s", e);
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}

fn prompt_password() -> Option<String> {
    print!("Enter slopcoder password: ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return None;
    }
    let trimmed = input.trim_end_matches(&['\r', '\n'][..]).to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn default_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn normalize_server_url(input: &str) -> String {
    let trimmed = input.trim_end_matches('/');
    if trimmed.ends_with("/agent/connect") {
        return trimmed.to_string();
    }
    format!("{}/agent/connect", trimmed)
}

async fn run_connection(
    state: AppState,
    server_url: &str,
    password: Option<String>,
    hostname: String,
    display_name: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut request = server_url.into_client_request()?;
    if let Some(password) = password {
        request.headers_mut().insert(
            "x-slopcoder-password",
            password
                .parse()
                .map_err(|e| format!("invalid password header: {e}"))?,
        );
    }
    let (ws_stream, _) = connect_async(request).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    ws_sink
        .send(Message::Text(
            serde_json::to_string(&AgentEnvelope::Hello {
                hostname,
                display_name,
            })?
            .into(),
        ))
        .await?;

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<AgentEnvelope>();
    let writer = tokio::spawn(async move {
        while let Some(envelope) = out_rx.recv().await {
            let payload = match serde_json::to_string(&envelope) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Failed to serialize outgoing envelope: {}", e);
                    continue;
                }
            };
            if ws_sink.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = ws_stream.next().await {
        let message = message?;
        if message.is_close() {
            break;
        }
        if !message.is_text() {
            continue;
        }

        let text = message.into_text()?;
        let envelope: AgentEnvelope = match serde_json::from_str(&text) {
            Ok(env) => env,
            Err(e) => {
                tracing::warn!("Failed to parse coordinator message: {}", e);
                continue;
            }
        };

        match envelope {
            AgentEnvelope::Request {
                request_id,
                request,
            } => {
                let response = handle_request(state.clone(), request, out_tx.clone()).await;
                let outgoing = match response {
                    Ok(response) => AgentEnvelope::Response {
                        request_id,
                        response,
                    },
                    Err(err) => AgentEnvelope::Error {
                        request_id,
                        status: err.status,
                        error: err.error,
                    },
                };
                if out_tx.send(outgoing).is_err() {
                    break;
                }
            }
            _ => {
                tracing::warn!("Ignoring unexpected envelope from coordinator");
            }
        }
    }

    writer.abort();
    Ok(())
}

async fn handle_request(
    state: AppState,
    request: AgentRequest,
    out_tx: mpsc::UnboundedSender<AgentEnvelope>,
) -> Result<AgentResponse, RpcError> {
    match request {
        AgentRequest::ListEnvironments => {
            let config = state.get_config().await;
            Ok(AgentResponse::Environments {
                environments: config.environments,
            })
        }
        AgentRequest::CreateEnvironment { name } => create_environment(state, &name).await,
        AgentRequest::ListBranches { environment } => list_branches(state, &environment).await,
        AgentRequest::ListTasks => Ok(AgentResponse::Tasks {
            tasks: state.list_tasks().await,
        }),
        AgentRequest::GetTask { task_id } => Ok(AgentResponse::Task {
            task: state.get_task(task_id).await,
        }),
        AgentRequest::CreateTask { request } => create_task(state, request, out_tx).await,
        AgentRequest::SendPrompt { task_id, prompt } => {
            send_prompt(state, task_id, prompt, out_tx).await
        }
        AgentRequest::GetTaskOutput { task_id } => get_task_output(state, task_id).await,
        AgentRequest::GetTaskDiff { task_id } => get_task_diff(state, task_id).await,
        AgentRequest::InterruptTask { task_id } => interrupt_task(state, task_id).await,
        AgentRequest::MergeTask { task_id } => merge_task(state, task_id).await,
    }
}

async fn create_environment(state: AppState, raw_name: &str) -> Result<AgentResponse, RpcError> {
    let name = raw_name.trim();
    if name.is_empty() {
        return Err(RpcError::new(
            StatusCode::BAD_REQUEST,
            "Environment name cannot be empty",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RpcError::new(
            StatusCode::BAD_REQUEST,
            "Environment name can only contain letters, numbers, hyphens, and underscores",
        ));
    }

    match state.create_new_environment(name).await {
        Ok(environment) => Ok(AgentResponse::Environment { environment }),
        Err(e) => {
            let status = match e {
                slopcoder_core::environment::EnvironmentError::AlreadyExists(_) => {
                    StatusCode::CONFLICT
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(RpcError::new(status, e.to_string()))
        }
    }
}

async fn list_branches(state: AppState, name: &str) -> Result<AgentResponse, RpcError> {
    let config = state.get_config().await;
    let Some(env) = config.find(name) else {
        return Err(RpcError::new(
            StatusCode::NOT_FOUND,
            format!("Environment '{}' not found", name),
        ));
    };
    match env.list_branches().await {
        Ok(branches) => Ok(AgentResponse::Branches { branches }),
        Err(e) => Err(RpcError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )),
    }
}

async fn create_task(
    state: AppState,
    req: AgentCreateTaskRequest,
    out_tx: mpsc::UnboundedSender<AgentEnvelope>,
) -> Result<AgentResponse, RpcError> {
    let config = state.get_config().await;
    let Some(env) = config.find(&req.environment) else {
        return Err(RpcError::new(
            StatusCode::NOT_FOUND,
            format!("Environment '{}' not found", req.environment),
        ));
    };

    let base_branch = req
        .base_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_string();

    let feature_branch = match req
        .feature_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(branch) => branch.to_string(),
        None => {
            let model = state.get_branch_model().await;
            match pick_feature_branch(&req.prompt, &model).await {
                Ok(branch) => branch,
                Err(_) => {
                    return Err(RpcError::new(
                        StatusCode::BAD_REQUEST,
                        "You must enter the name of the feature branch.",
                    ));
                }
            }
        }
    };

    let worktree_path = match env
        .create_worktree_from_base(&base_branch, &feature_branch)
        .await
    {
        Ok(path) => path,
        Err(e) => {
            let status = match e {
                slopcoder_core::environment::EnvironmentError::BranchExists(_)
                | slopcoder_core::environment::EnvironmentError::WorktreeExists(_) => {
                    StatusCode::CONFLICT
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return Err(RpcError::new(
                status,
                format!("Failed to create worktree: {}", e),
            ));
        }
    };

    let task = Task::new(
        req.agent.unwrap_or_default(),
        req.environment,
        Some(base_branch),
        feature_branch,
        worktree_path.clone(),
    );
    let task_id = task.id;

    state
        .insert_task(task)
        .await
        .map_err(|e| RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let prompt = req.prompt;
    let state_clone = state.clone();
    tokio::spawn(async move {
        run_agent(state_clone, task_id, prompt, None, out_tx).await;
    });

    Ok(AgentResponse::CreatedTask {
        id: task_id,
        worktree_path: worktree_path.to_string_lossy().to_string(),
    })
}

async fn send_prompt(
    state: AppState,
    task_id: TaskId,
    prompt: String,
    out_tx: mpsc::UnboundedSender<AgentEnvelope>,
) -> Result<AgentResponse, RpcError> {
    let Some(task) = state.get_task(task_id).await else {
        return Err(RpcError::new(StatusCode::NOT_FOUND, "Task not found"));
    };

    if !task.can_run() {
        return Err(RpcError::new(
            StatusCode::CONFLICT,
            "Task is currently running",
        ));
    }

    if !state.validate_task_worktree(task_id).await {
        return Err(RpcError::new(
            StatusCode::GONE,
            "Task worktree no longer exists (may have been removed from CLI)",
        ));
    }

    let session_id = task.session_id;
    let state_clone = state.clone();
    tokio::spawn(async move {
        run_agent(state_clone, task_id, prompt, session_id, out_tx).await;
    });

    Ok(AgentResponse::Ack)
}

async fn interrupt_task(state: AppState, task_id: TaskId) -> Result<AgentResponse, RpcError> {
    if state.send_interrupt(task_id).await {
        Ok(AgentResponse::Ack)
    } else {
        Err(RpcError::new(
            StatusCode::CONFLICT,
            "Task is not running or interrupt channel not found",
        ))
    }
}

async fn get_task_output(state: AppState, task_id: TaskId) -> Result<AgentResponse, RpcError> {
    let Some(task) = state.get_task(task_id).await else {
        return Err(RpcError::new(StatusCode::NOT_FOUND, "Task not found"));
    };

    let Some(env_dir) = state.get_environment_directory(&task.environment).await else {
        return Err(RpcError::new(
            StatusCode::NOT_FOUND,
            "Environment not found",
        ));
    };

    let output_path = task_output_path(&env_dir, task_id);
    let events = read_output_events(&output_path)
        .await
        .map_err(|e| RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(AgentResponse::TaskOutput { events })
}

async fn get_task_diff(state: AppState, task_id: TaskId) -> Result<AgentResponse, RpcError> {
    let Some(task) = state.get_task(task_id).await else {
        return Err(RpcError::new(StatusCode::NOT_FOUND, "Task not found"));
    };
    let Some(base_branch) = task.base_branch.as_deref() else {
        return Err(RpcError::new(
            StatusCode::BAD_REQUEST,
            "Task has no base branch",
        ));
    };
    let diff = load_git_diff(&task.worktree_path, base_branch)
        .await
        .map_err(|e| RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(AgentResponse::TaskDiff {
        staged: diff.staged,
        unstaged: diff.unstaged,
    })
}

async fn merge_task(state: AppState, task_id: TaskId) -> Result<AgentResponse, RpcError> {
    let Some(task) = state.get_task(task_id).await else {
        return Err(RpcError::new(StatusCode::NOT_FOUND, "Task not found"));
    };

    if has_unstaged_changes(&task.worktree_path).await {
        return Err(RpcError::new(
            StatusCode::CONFLICT,
            "Task has unstaged changes. Please commit or stash them before merging.",
        ));
    }

    let base_branch = task.base_branch.as_deref().unwrap_or("main");
    let config = state.get_config().await;
    let Some(env) = config.find(&task.environment) else {
        return Err(RpcError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Environment not found in config",
        ));
    };

    let target_worktree = env.worktree_path(base_branch);
    if target_worktree.exists() {
        if has_unstaged_changes(&target_worktree).await {
            return Err(RpcError::new(
                StatusCode::CONFLICT,
                format!(
                    "Target branch '{}' worktree has unstaged changes.",
                    base_branch
                ),
            ));
        }
    } else if let Err(e) = env.create_worktree(base_branch).await {
        return Err(RpcError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create worktree for '{}': {}", base_branch, e),
        ));
    }

    let merge_output = Command::new("git")
        .args(["merge", &task.feature_branch])
        .current_dir(&target_worktree)
        .output()
        .await
        .map_err(|e| RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if merge_output.status.success() {
        Ok(AgentResponse::MergeResult {
            status: "merged".to_string(),
            message: format!(
                "Successfully merged {} into {}",
                task.feature_branch, base_branch
            ),
        })
    } else {
        let stdout = String::from_utf8_lossy(&merge_output.stdout);
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(&target_worktree)
            .output()
            .await;

        Err(RpcError::new(
            StatusCode::CONFLICT,
            format!("Merge failed (reverted): {}", stdout.trim()),
        ))
    }
}

async fn run_agent(
    state: AppState,
    task_id: TaskId,
    prompt: String,
    session_id: Option<Uuid>,
    event_tx: mpsc::UnboundedSender<AgentEnvelope>,
) {
    let task = match state.get_task(task_id).await {
        Some(t) => t,
        None => {
            tracing::error!("Task {} not found", task_id);
            return;
        }
    };

    let mut output_file = match state.get_environment_directory(&task.environment).await {
        Some(env_dir) => {
            let output_path = task_output_path(&env_dir, task_id);
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&output_path)
                .await
            {
                Ok(file) => Some(file),
                Err(e) => {
                    tracing::warn!("Failed to open output log {}: {}", output_path.display(), e);
                    None
                }
            }
        }
        None => None,
    };

    if let Err(e) = state.start_task_run(task_id, prompt.clone()).await {
        tracing::error!("Failed to start task run for {}: {}", task_id, e);
        return;
    }

    let mut interrupt_rx = state.register_interrupt_channel(task_id).await;
    let agent_config = state.get_agent_config().await;

    let prompt_event = AgentEvent::PromptSent {
        prompt: prompt.clone(),
    };
    if let Some(file) = output_file.as_mut() {
        if let Ok(line) = serde_json::to_string(&prompt_event) {
            if file.write_all(line.as_bytes()).await.is_err()
                || file.write_all(b"\n").await.is_err()
            {
                output_file = None;
            }
        }
    }
    let _ = event_tx.send(AgentEnvelope::TaskEvent {
        task_id,
        event: prompt_event,
    });

    let agent_result = if let Some(sid) = session_id {
        resume_anyagent(task.agent, &agent_config, &task.worktree_path, sid, &prompt).await
    } else {
        spawn_anyagent(task.agent, &agent_config, &task.worktree_path, &prompt).await
    };

    let mut agent = match agent_result {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to spawn agent: {}", e);
            let _ = state.complete_task_run(task_id, false).await;
            return;
        }
    };

    let mut interrupted = false;
    loop {
        tokio::select! {
            result = agent.next_event() => {
                match result {
                    Some(Ok(event)) => {
                        if let Some(sid) = event.session_id() {
                            if let Err(e) = state.set_task_session_id(task_id, sid).await {
                                tracing::warn!("Failed to save session ID: {}", e);
                            }
                        }
                        if let Some(file) = output_file.as_mut() {
                            match serde_json::to_string(&event) {
                                Ok(line) => {
                                    if file.write_all(line.as_bytes()).await.is_err()
                                        || file.write_all(b"\n").await.is_err()
                                    {
                                        output_file = None;
                                    }
                                }
                                Err(e) => tracing::warn!("Failed to serialize event for {}: {}", task_id, e),
                            }
                        }
                        let _ = event_tx.send(AgentEnvelope::TaskEvent { task_id, event });
                    }
                    Some(Err(e)) => tracing::warn!("Error reading event: {}", e),
                    None => break,
                }
            }
            _ = &mut interrupt_rx => {
                interrupted = true;
                if let Err(e) = agent.kill().await {
                    tracing::warn!("Failed to kill agent for task {}: {}", task_id, e);
                }
                break;
            }
        }
    }

    if interrupted {
        if let Err(e) = state.interrupt_task_run(task_id).await {
            tracing::warn!("Failed to persist interrupt for {}: {}", task_id, e);
        }
    } else {
        let result = agent.wait().await;
        let success = match &result {
            Ok(r) => {
                if let Err(e) = state.set_task_session_id(task_id, r.session_id).await {
                    tracing::warn!("Failed to save session ID: {}", e);
                }
                r.success
            }
            Err(_) => false,
        };

        if let Err(e) = state.complete_task_run(task_id, success).await {
            tracing::warn!("Failed to persist completion for {}: {}", task_id, e);
        }
    }
}

fn task_output_path(env_dir: &Path, task_id: TaskId) -> PathBuf {
    env_dir.join(format!("task-{}.jsonl", task_id))
}

async fn read_output_events(path: &Path) -> Result<Vec<AgentEvent>, std::io::Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path).await?;
    let mut lines = BufReader::new(file).lines();
    let mut events = Vec::new();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEvent>(trimmed) {
            events.push(event);
        }
    }

    Ok(events)
}

struct DiffResult {
    staged: String,
    unstaged: String,
}

async fn load_git_diff(
    worktree_path: &Path,
    base_branch: &str,
) -> Result<DiffResult, std::io::Error> {
    let staged_output = Command::new("git")
        .args(["diff", "--cached", base_branch])
        .current_dir(worktree_path)
        .output()
        .await?;
    if !staged_output.status.success() {
        let stderr = String::from_utf8_lossy(&staged_output.stderr);
        if !stderr.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                stderr.to_string(),
            ));
        }
    }
    let staged = String::from_utf8_lossy(&staged_output.stdout).to_string();

    let unstaged_output = Command::new("git")
        .args(["diff"])
        .current_dir(worktree_path)
        .output()
        .await?;
    if !unstaged_output.status.success() {
        let stderr = String::from_utf8_lossy(&unstaged_output.stderr);
        if !stderr.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                stderr.to_string(),
            ));
        }
    }
    let mut unstaged = String::from_utf8_lossy(&unstaged_output.stdout).to_string();

    let untracked = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .await?;
    if !untracked.status.success() {
        let stderr = String::from_utf8_lossy(&untracked.stderr);
        if !stderr.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                stderr.to_string(),
            ));
        }
    }

    for path in String::from_utf8_lossy(&untracked.stdout).lines() {
        if path.trim().is_empty() {
            continue;
        }
        let untracked_diff = Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .current_dir(worktree_path)
            .output()
            .await?;
        let status = untracked_diff.status;
        if !status.success() && status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&untracked_diff.stderr);
            if !stderr.trim().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    stderr.to_string(),
                ));
            }
        }

        let chunk = String::from_utf8_lossy(&untracked_diff.stdout);
        if !chunk.trim().is_empty() {
            if !unstaged.is_empty() && !unstaged.ends_with('\n') {
                unstaged.push('\n');
            }
            unstaged.push_str(&chunk);
        }
    }

    Ok(DiffResult { staged, unstaged })
}

async fn has_unstaged_changes(worktree_path: &Path) -> bool {
    let unstaged_output = Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(worktree_path)
        .output()
        .await;
    if let Ok(output) = unstaged_output {
        if output.status.code() == Some(1) {
            return true;
        }
    }

    let staged_output = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(worktree_path)
        .output()
        .await;
    if let Ok(output) = staged_output {
        if output.status.code() == Some(1) {
            return true;
        }
    }

    let untracked_output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .await;
    if let Ok(output) = untracked_output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            return true;
        }
    }

    false
}

#[allow(dead_code)]
fn map_state_error(err: StateError) -> RpcError {
    match err {
        StateError::TaskNotFound(_) => RpcError::new(StatusCode::NOT_FOUND, "Task not found"),
        StateError::WorktreeMissing(_) => RpcError::new(StatusCode::GONE, "Task worktree missing"),
        StateError::TaskNotReady => RpcError::new(StatusCode::CONFLICT, "Task not ready"),
        StateError::PersistenceError(e) => {
            RpcError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

#[allow(dead_code)]
fn map_ws_error(err: tungstenite::Error) -> String {
    err.to_string()
}
