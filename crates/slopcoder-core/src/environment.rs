//! Environment configuration and management.
//!
//! Each environment maps to a checked-out Git repository directory.
//! Optional isolated task worktrees are created in a shared worktrees directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;

/// Errors that can occur when working with environments.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error("Failed to read configuration file: {0}")]
    ConfigReadError(#[from] std::io::Error),

    #[error("Environment '{0}' not found")]
    NotFound(String),

    #[error("Environment '{0}' already exists")]
    AlreadyExists(String),

    #[error("Repository directory does not exist: {0}")]
    RepositoryNotFound(PathBuf),

    #[error("Directory is not a checked-out Git repository: {0}")]
    InvalidGitRepository(PathBuf),

    #[error("Failed to list branches: {0}")]
    BranchListError(String),

    #[error("Failed to check branch: {0}")]
    BranchCheckError(String),

    #[error("Failed to resolve current branch: {0}")]
    CurrentBranchError(String),

    #[error("Failed to create worktree: {0}")]
    WorktreeCreateError(String),

    #[error("Branch already exists: {0}")]
    BranchExists(String),

    #[error("Worktree already exists at {0}")]
    WorktreeExists(PathBuf),

    #[error("Worktrees directory does not exist or is not a directory: {0}")]
    WorktreesDirInvalid(PathBuf),
}

/// In-memory environment configuration.
#[derive(Debug, Clone)]
pub struct EnvironmentConfig {
    /// Directory where isolated task worktrees are created.
    pub worktrees_directory: PathBuf,
    /// Root directory scanned for additional checked-out repositories.
    pub environments_root: PathBuf,
    /// Configured repository environments.
    pub environments: Vec<Environment>,
}

impl EnvironmentConfig {
    pub fn new(
        worktrees_directory: PathBuf,
        environments_root: Option<PathBuf>,
        environments: Vec<PathBuf>,
    ) -> Self {
        Self {
            worktrees_directory,
            environments_root: environments_root.unwrap_or_else(Self::default_environments_root),
            environments: environments
                .into_iter()
                .map(|directory| Environment {
                    name: directory.to_string_lossy().to_string(),
                    directory,
                })
                .collect(),
        }
    }

    /// Validate the worktrees directory exists and is a directory.
    pub async fn validate_worktrees_directory(&self) -> Result<(), EnvironmentError> {
        let path = &self.worktrees_directory;
        match tokio::fs::metadata(path).await {
            Ok(meta) if meta.is_dir() => Ok(()),
            _ => Err(EnvironmentError::WorktreesDirInvalid(path.clone())),
        }
    }

    /// Find an environment by name.
    pub fn find(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }

    pub fn default_environments_root() -> PathBuf {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("slop")
    }
}

/// An environment definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    /// Unique environment identifier.
    pub name: String,
    /// Checked-out repository directory.
    pub directory: PathBuf,
}

impl Environment {
    /// Validate that the environment directory is a checked-out git repository.
    pub async fn validate(&self) -> Result<(), EnvironmentError> {
        if !self.directory.exists() || !self.directory.is_dir() {
            return Err(EnvironmentError::RepositoryNotFound(self.directory.clone()));
        }

        let output = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::InvalidGitRepository(PathBuf::from(e.to_string())))?;

        if !output.status.success() {
            return Err(EnvironmentError::InvalidGitRepository(
                self.directory.clone(),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() != "true" {
            return Err(EnvironmentError::InvalidGitRepository(
                self.directory.clone(),
            ));
        }

        Ok(())
    }

    /// List all local branches in the repository.
    pub async fn list_branches(&self) -> Result<Vec<String>, EnvironmentError> {
        let output = Command::new("git")
            .args(["branch", "--list", "--format=%(refname:short)"])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::BranchListError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::BranchListError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();

        Ok(branches)
    }

    /// Resolve the current branch checked out in this environment repository.
    pub async fn current_branch(&self) -> Result<String, EnvironmentError> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::CurrentBranchError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::CurrentBranchError(stderr.to_string()));
        }

        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() || branch == "HEAD" {
            return Err(EnvironmentError::CurrentBranchError(
                "Repository is in detached HEAD state".to_string(),
            ));
        }

        Ok(branch)
    }

    fn environment_worktree_root(&self, worktrees_directory: &Path) -> PathBuf {
        worktrees_directory.join(sanitize_for_path(&self.name))
    }

    /// Get the path where a worktree for the given branch would be created.
    pub fn worktree_path(&self, worktrees_directory: &Path, branch: &str) -> PathBuf {
        self.environment_worktree_root(worktrees_directory)
            .join(sanitize_for_path(branch))
    }

    /// Check if a branch exists in the repository.
    pub async fn branch_exists(&self, branch: &str) -> Result<bool, EnvironmentError> {
        let ref_name = format!("refs/heads/{}", branch);

        let output = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &ref_name])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::BranchCheckError(e.to_string()))?;

        if output.status.success() {
            Ok(true)
        } else if output.status.code() == Some(1) {
            Ok(false)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(EnvironmentError::BranchCheckError(stderr.to_string()))
        }
    }

    /// Create a new worktree for an existing branch.
    pub async fn create_worktree(
        &self,
        worktrees_directory: &Path,
        branch: &str,
    ) -> Result<PathBuf, EnvironmentError> {
        let worktree_path = self.worktree_path(worktrees_directory, branch);

        if worktree_path.exists() {
            return Err(EnvironmentError::WorktreeExists(worktree_path));
        }

        if let Some(parent) = worktree_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| EnvironmentError::WorktreeCreateError(e.to_string()))?;
        }

        let output = Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap(), branch])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::WorktreeCreateError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::WorktreeCreateError(stderr.to_string()));
        }

        Ok(worktree_path)
    }

    /// Create a new worktree for a new branch based on a base branch.
    pub async fn create_worktree_from_base(
        &self,
        worktrees_directory: &Path,
        base_branch: &str,
        feature_branch: &str,
    ) -> Result<PathBuf, EnvironmentError> {
        let worktree_path = self.worktree_path(worktrees_directory, feature_branch);

        if worktree_path.exists() {
            return Err(EnvironmentError::WorktreeExists(worktree_path));
        }

        if self.branch_exists(feature_branch).await? {
            return Err(EnvironmentError::BranchExists(feature_branch.to_string()));
        }

        if let Some(parent) = worktree_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| EnvironmentError::WorktreeCreateError(e.to_string()))?;
        }

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                feature_branch,
                worktree_path.to_str().unwrap(),
                base_branch,
            ])
            .current_dir(&self.directory)
            .output()
            .await
            .map_err(|e| EnvironmentError::WorktreeCreateError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::WorktreeCreateError(stderr.to_string()));
        }

        Ok(worktree_path)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_config() {
        let config = EnvironmentConfig::new(
            PathBuf::from("/tmp/slopcoder-worktrees"),
            Some(PathBuf::from("/tmp/slop")),
            vec![
                PathBuf::from("/tmp/test-project"),
                PathBuf::from("/home/user/projects/another"),
            ],
        );
        assert_eq!(config.environments.len(), 2);
        assert_eq!(config.environments_root, PathBuf::from("/tmp/slop"));
        assert_eq!(
            config.environments[0].name,
            PathBuf::from("/tmp/test-project").to_string_lossy()
        );
        assert_eq!(
            config.environments[0].directory,
            PathBuf::from("/tmp/test-project")
        );
    }

    #[test]
    fn test_find_environment() {
        let config = EnvironmentConfig::new(
            PathBuf::from("/tmp/slopcoder-worktrees"),
            Some(PathBuf::from("/tmp/slop")),
            vec![PathBuf::from("/tmp/test-project")],
        );
        let env_name = PathBuf::from("/tmp/test-project")
            .to_string_lossy()
            .to_string();
        let env = config.find(&env_name).unwrap();
        assert_eq!(env.directory, PathBuf::from("/tmp/test-project"));

        assert!(config.find("nonexistent").is_none());
    }

    #[test]
    fn test_worktree_path() {
        let env = Environment {
            name: "/tmp/test-project".to_string(),
            directory: PathBuf::from("/tmp/test-project"),
        };
        assert_eq!(
            env.worktree_path(Path::new("/tmp/worktrees"), "main"),
            PathBuf::from("/tmp/worktrees/tmp-test-project/main")
        );
        assert_eq!(
            env.worktree_path(Path::new("/tmp/worktrees"), "feature/foo"),
            PathBuf::from("/tmp/worktrees/tmp-test-project/feature-foo")
        );
    }

    #[test]
    fn test_default_environments_root() {
        let root = EnvironmentConfig::default_environments_root();
        assert!(root.ends_with("slop"));
    }
}
