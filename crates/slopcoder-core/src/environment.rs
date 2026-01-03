//! Environment configuration and management.
//!
//! An environment represents a project with a bare Git repository
//! and multiple worktrees for running agent tasks.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;

/// Errors that can occur when working with environments.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error("Failed to read configuration file: {0}")]
    ConfigReadError(#[from] std::io::Error),

    #[error("Failed to parse configuration: {0}")]
    ConfigParseError(#[from] serde_yaml::Error),

    #[error("Environment '{0}' not found")]
    NotFound(String),

    #[error("Bare repository not found at {0}")]
    BareRepoNotFound(PathBuf),

    #[error("Failed to list branches: {0}")]
    BranchListError(String),

    #[error("Failed to create worktree: {0}")]
    WorktreeCreateError(String),

    #[error("Worktree already exists at {0}")]
    WorktreeExists(PathBuf),
}

/// Configuration file format for environments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    pub environments: Vec<Environment>,
}

impl EnvironmentConfig {
    /// Load configuration from a YAML file.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let content = tokio::fs::read_to_string(path).await?;
        let config: EnvironmentConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, EnvironmentError> {
        let config: EnvironmentConfig = serde_yaml::from_str(yaml)?;
        Ok(config)
    }

    /// Find an environment by name.
    pub fn find(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }
}

/// An environment definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    /// Unique name for this environment.
    pub name: String,
    /// Base directory containing the bare repo and worktrees.
    pub directory: PathBuf,
}

impl Environment {
    /// Get the path to the bare repository.
    pub fn bare_repo_path(&self) -> PathBuf {
        self.directory.join("bare")
    }

    /// Validate that the environment is properly set up.
    pub async fn validate(&self) -> Result<(), EnvironmentError> {
        let bare_path = self.bare_repo_path();
        if !bare_path.exists() {
            return Err(EnvironmentError::BareRepoNotFound(bare_path));
        }
        // Check it's actually a git repo
        let head_path = bare_path.join("HEAD");
        if !head_path.exists() {
            return Err(EnvironmentError::BareRepoNotFound(bare_path));
        }
        Ok(())
    }

    /// List all branches in the bare repository.
    pub async fn list_branches(&self) -> Result<Vec<String>, EnvironmentError> {
        let bare_path = self.bare_repo_path();

        let output = Command::new("git")
            .args(["branch", "--list", "--format=%(refname:short)"])
            .current_dir(&bare_path)
            .output()
            .await
            .map_err(|e| EnvironmentError::BranchListError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::BranchListError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let branches: Vec<String> = stdout.lines().map(|s| s.trim().to_string()).collect();

        Ok(branches)
    }

    /// Get the path where a worktree for the given branch would be created.
    pub fn worktree_path(&self, branch: &str) -> PathBuf {
        // Sanitize branch name for use as directory
        let safe_name = branch.replace('/', "-");
        self.directory.join(&safe_name)
    }

    /// Create a new worktree for the given branch.
    pub async fn create_worktree(&self, branch: &str) -> Result<PathBuf, EnvironmentError> {
        let worktree_path = self.worktree_path(branch);

        if worktree_path.exists() {
            return Err(EnvironmentError::WorktreeExists(worktree_path));
        }

        let bare_path = self.bare_repo_path();

        let output = Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap(), branch])
            .current_dir(&bare_path)
            .output()
            .await
            .map_err(|e| EnvironmentError::WorktreeCreateError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::WorktreeCreateError(stderr.to_string()));
        }

        Ok(worktree_path)
    }

    /// Check if a worktree exists for the given branch.
    pub fn worktree_exists(&self, branch: &str) -> bool {
        self.worktree_path(branch).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONFIG: &str = r#"
environments:
  - name: "test-project"
    directory: "/tmp/test-project"
  - name: "another-project"
    directory: "/home/user/projects/another"
"#;

    #[test]
    fn test_parse_config() {
        let config = EnvironmentConfig::from_yaml(SAMPLE_CONFIG).unwrap();
        assert_eq!(config.environments.len(), 2);
        assert_eq!(config.environments[0].name, "test-project");
        assert_eq!(
            config.environments[0].directory,
            PathBuf::from("/tmp/test-project")
        );
    }

    #[test]
    fn test_find_environment() {
        let config = EnvironmentConfig::from_yaml(SAMPLE_CONFIG).unwrap();
        let env = config.find("test-project").unwrap();
        assert_eq!(env.name, "test-project");

        assert!(config.find("nonexistent").is_none());
    }

    #[test]
    fn test_bare_repo_path() {
        let env = Environment {
            name: "test".to_string(),
            directory: PathBuf::from("/tmp/test-project"),
        };
        assert_eq!(env.bare_repo_path(), PathBuf::from("/tmp/test-project/bare"));
    }

    #[test]
    fn test_worktree_path() {
        let env = Environment {
            name: "test".to_string(),
            directory: PathBuf::from("/tmp/test-project"),
        };
        assert_eq!(
            env.worktree_path("main"),
            PathBuf::from("/tmp/test-project/main")
        );
        // Branches with slashes get converted
        assert_eq!(
            env.worktree_path("feature/foo"),
            PathBuf::from("/tmp/test-project/feature-foo")
        );
    }
}
