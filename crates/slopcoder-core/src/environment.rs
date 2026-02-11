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

    #[error("Failed to parse configuration: {0}")]
    ConfigParseError(#[from] serde_yaml::Error),

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

/// Configuration file format used for YAML serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentConfigFile {
    pub worktrees_directory: PathBuf,
    pub environments: Vec<PathBuf>,
}

/// In-memory environment configuration.
#[derive(Debug, Clone)]
pub struct EnvironmentConfig {
    /// Directory where isolated task worktrees are created.
    pub worktrees_directory: PathBuf,
    /// Configured repository environments.
    pub environments: Vec<Environment>,
}

impl EnvironmentConfig {
    /// Load configuration from a YAML file.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let content = tokio::fs::read_to_string(path).await?;
        Self::from_yaml(&content)
    }

    /// Load configuration from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, EnvironmentError> {
        let file: EnvironmentConfigFile = serde_yaml::from_str(yaml)?;
        Ok(Self {
            worktrees_directory: file.worktrees_directory,
            environments: file
                .environments
                .into_iter()
                .map(|directory| Environment {
                    name: directory.to_string_lossy().to_string(),
                    directory,
                })
                .collect(),
        })
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

    /// Save configuration to a YAML file.
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<(), EnvironmentError> {
        let file = EnvironmentConfigFile {
            worktrees_directory: self.worktrees_directory.clone(),
            environments: self
                .environments
                .iter()
                .map(|e| e.directory.clone())
                .collect(),
        };
        let content = serde_yaml::to_string(&file)?;
        tokio::fs::write(path, content).await?;
        Ok(())
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

    const SAMPLE_CONFIG: &str = r#"
worktrees_directory: "/tmp/slopcoder-worktrees"
environments:
  - "/tmp/test-project"
  - "/home/user/projects/another"
"#;

    #[test]
    fn test_parse_config() {
        let config = EnvironmentConfig::from_yaml(SAMPLE_CONFIG).unwrap();
        assert_eq!(config.environments.len(), 2);
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
        let config = EnvironmentConfig::from_yaml(SAMPLE_CONFIG).unwrap();
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
}
