use slopcoder_core::{
    anyagent::AnyAgentConfig,
    environment::{Environment, EnvironmentConfig, EnvironmentError},
    persistence::PersistentTaskStore,
    task::{Task, TaskId},
    PersistenceError,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::process::Command;
use tokio::sync::RwLock;
use uuid::Uuid;

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

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("Worktrees directory validation failed: {0}")]
    WorktreesDirValidation(EnvironmentError),

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

#[derive(Debug, Error)]
pub enum CreateEnvironmentError {
    #[error("Environment name is required")]
    NameRequired,

    #[error("Environment name cannot contain path separators")]
    InvalidName,

    #[error("Environment already exists: {0}")]
    AlreadyExists(PathBuf),

    #[error("Failed to create environment directory: {0}")]
    CreateDirectory(std::io::Error),

    #[error("Failed to initialize git repository: {0}")]
    GitInit(String),
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<AppStateInner>>,
}

struct AppStateInner {
    config: EnvironmentConfig,
    state_root: PathBuf,
    tasks: PersistentTaskStore,
    interrupt_channels: std::collections::HashMap<TaskId, tokio::sync::oneshot::Sender<()>>,
    agent_config: AnyAgentConfig,
    branch_model: String,
}

impl AppState {
    pub async fn new(
        config_path: PathBuf,
        branch_model: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config = EnvironmentConfig::load(&config_path).await?;

        if let Err(err) = config.validate_worktrees_directory().await {
            return Err(Box::new(StartupError::WorktreesDirValidation(err)));
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

        let mut tasks = PersistentTaskStore::new();
        let state_root = config.worktrees_directory.join(".slopcoder-state");
        tokio::fs::create_dir_all(&state_root).await?;
        let discovered = discover_environments_under(&config.environments_root).await;
        for env in config.environments.iter().chain(discovered.iter()) {
            let env_state_dir = state_root.join(sanitize_for_path(&env.name));
            tokio::fs::create_dir_all(&env_state_dir).await?;
            tasks.register_environment(env.name.clone(), env_state_dir);
        }
        tasks.load_all().await?;

        Ok(Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                config,
                state_root,
                tasks,
                interrupt_channels: std::collections::HashMap::new(),
                agent_config: AnyAgentConfig::default(),
                branch_model,
            })),
        })
    }

    pub async fn list_environments(&self) -> Vec<Environment> {
        let config = self.inner.read().await.config.clone();
        let mut by_name = std::collections::BTreeMap::new();
        for env in config.environments {
            by_name.insert(env.name.clone(), env);
        }
        for env in discover_environments_under(&config.environments_root).await {
            by_name.insert(env.name.clone(), env);
        }
        by_name.into_values().collect()
    }

    pub async fn find_environment(&self, name: &str) -> Option<Environment> {
        self.list_environments()
            .await
            .into_iter()
            .find(|env| env.name == name)
    }

    pub async fn create_environment(
        &self,
        raw_name: &str,
    ) -> Result<Environment, CreateEnvironmentError> {
        let name = raw_name.trim();
        if name.is_empty() {
            return Err(CreateEnvironmentError::NameRequired);
        }
        if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
            return Err(CreateEnvironmentError::InvalidName);
        }

        let root = self.inner.read().await.config.environments_root.clone();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(CreateEnvironmentError::CreateDirectory)?;

        let directory = root.join(name);
        if directory.exists() {
            return Err(CreateEnvironmentError::AlreadyExists(directory));
        }
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(CreateEnvironmentError::CreateDirectory)?;

        let init_output = Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(&directory)
            .output()
            .await
            .map_err(|e| CreateEnvironmentError::GitInit(e.to_string()))?;
        if !init_output.status.success() {
            let fallback_output = Command::new("git")
                .args(["init"])
                .current_dir(&directory)
                .output()
                .await
                .map_err(|e| CreateEnvironmentError::GitInit(e.to_string()))?;
            if !fallback_output.status.success() {
                let stderr = String::from_utf8_lossy(&fallback_output.stderr).to_string();
                return Err(CreateEnvironmentError::GitInit(stderr));
            }
        }

        let commit_output = Command::new("git")
            .args([
                "-c",
                "user.name=slopcoder",
                "-c",
                "user.email=slopcoder@local",
                "commit",
                "--allow-empty",
                "-m",
                "Initialize repository",
            ])
            .current_dir(&directory)
            .output()
            .await
            .map_err(|e| CreateEnvironmentError::GitInit(e.to_string()))?;
        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr).to_string();
            return Err(CreateEnvironmentError::GitInit(stderr));
        }

        let env = Environment {
            name: directory.to_string_lossy().to_string(),
            directory,
        };
        self.ensure_environment_registered(&env.name)
            .await
            .map_err(|e| CreateEnvironmentError::GitInit(e.to_string()))?;
        Ok(env)
    }

    pub async fn ensure_environment_registered(&self, env_name: &str) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if inner.tasks.has_environment(env_name) {
            return Ok(());
        }

        let env_state_dir = inner.state_root.join(sanitize_for_path(env_name));
        inner
            .tasks
            .register_environment(env_name.to_string(), env_state_dir);
        Ok(())
    }

    pub async fn get_environment_directory(&self, name: &str) -> Option<PathBuf> {
        self.inner
            .read()
            .await
            .tasks
            .get_environment_directory(name)
    }

    pub async fn get_worktrees_directory(&self) -> PathBuf {
        self.inner.read().await.config.worktrees_directory.clone()
    }

    pub async fn get_agent_config(&self) -> AnyAgentConfig {
        self.inner.read().await.agent_config.clone()
    }

    pub async fn get_branch_model(&self) -> String {
        self.inner.read().await.branch_model.clone()
    }

    pub async fn list_tasks(&self) -> Vec<Task> {
        {
            let mut inner = self.inner.write().await;
            if let Err(e) = inner.tasks.cleanup_stale_tasks().await {
                tracing::warn!("Failed to cleanup stale tasks: {}", e);
            }
        }

        self.inner
            .read()
            .await
            .tasks
            .list()
            .into_iter()
            .cloned()
            .collect()
    }

    pub async fn get_task(&self, id: TaskId) -> Option<Task> {
        self.inner.read().await.tasks.get(id).cloned()
    }

    pub async fn validate_task_worktree(&self, id: TaskId) -> bool {
        self.inner.read().await.tasks.validate_task_worktree(id)
    }

    pub async fn insert_task(&self, task: Task) -> Result<(), StateError> {
        self.ensure_environment_registered(&task.environment)
            .await?;
        self.inner.write().await.tasks.insert(task).await?;
        Ok(())
    }

    pub async fn set_task_session_id(
        &self,
        id: TaskId,
        session_id: Uuid,
    ) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.session_id = Some(session_id);
            inner.tasks.save_task(id).await?;
            Ok(())
        } else {
            Err(StateError::TaskNotFound(id))
        }
    }

    pub async fn start_task_run(&self, id: TaskId, prompt: String) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;

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

    pub async fn complete_task_run(&self, id: TaskId, success: bool) -> Result<(), StateError> {
        let mut inner = self.inner.write().await;
        if let Some(task) = inner.tasks.get_mut(id) {
            task.complete_run(success);
            inner.tasks.save_task(id).await?;
        }
        inner.interrupt_channels.remove(&id);
        Ok(())
    }

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
        inner.interrupt_channels.remove(&id);
        Ok(())
    }

    pub async fn register_interrupt_channel(
        &self,
        id: TaskId,
    ) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner.write().await.interrupt_channels.insert(id, tx);
        rx
    }

    pub async fn send_interrupt(&self, id: TaskId) -> bool {
        if let Some(tx) = self.inner.write().await.interrupt_channels.remove(&id) {
            tx.send(()).is_ok()
        } else {
            false
        }
    }
}

async fn discover_environments_under(root: &Path) -> Vec<Environment> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    for candidate in candidate_paths(root).await {
        if seen.insert(candidate.clone()) && is_checked_out_repo(&candidate).await {
            paths.push(candidate);
        }
    }

    paths.sort();
    paths
        .into_iter()
        .map(|directory| Environment {
            name: directory.to_string_lossy().to_string(),
            directory,
        })
        .collect()
}

async fn candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if !root.is_dir() {
        return candidates;
    }

    candidates.push(root.to_path_buf());
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(_) => return candidates,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        candidates.push(path.clone());
        candidates.push(path.join("main"));
        candidates.push(path.join("master"));
    }

    candidates
}

async fn is_checked_out_repo(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let output = match Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .await
    {
        Ok(output) => output,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).trim() == "true"
}

fn sanitize_for_path(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' || lower == '_' {
            out.push(lower);
        } else {
            out.push('-');
        }
    }

    let compact = out
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if compact.is_empty() {
        "env".to_string()
    } else {
        compact
    }
}
