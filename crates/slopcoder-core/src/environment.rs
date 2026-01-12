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

    #[error("Environment '{0}' already exists")]
    AlreadyExists(String),

    #[error("Bare repository not found at {0}")]
    BareRepoNotFound(PathBuf),

    #[error("Failed to list branches: {0}")]
    BranchListError(String),

    #[error("Failed to check branch: {0}")]
    BranchCheckError(String),

    #[error("Failed to create worktree: {0}")]
    WorktreeCreateError(String),

    #[error("Branch already exists: {0}")]
    BranchExists(String),

    #[error("Worktree already exists at {0}")]
    WorktreeExists(PathBuf),

    #[error("New environments directory does not exist or is not a directory: {0}")]
    NewEnvDirInvalid(PathBuf),

    #[error("Failed to initialize git repository: {0}")]
    GitInitError(String),
}

/// Configuration file format for environments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentConfig {
    /// Directory where new environments are created.
    pub new_environments_directory: PathBuf,
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

    /// Validate the new_environments_directory exists and is a directory.
    pub async fn validate_new_environments_directory(&self) -> Result<(), EnvironmentError> {
        let path = &self.new_environments_directory;
        match tokio::fs::metadata(path).await {
            Ok(meta) if meta.is_dir() => Ok(()),
            _ => Err(EnvironmentError::NewEnvDirInvalid(path.clone())),
        }
    }

    /// Find an environment by name.
    pub fn find(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }

    /// Add a new environment to the configuration.
    pub fn add_environment(&mut self, env: Environment) -> Result<(), EnvironmentError> {
        if self.find(&env.name).is_some() {
            return Err(EnvironmentError::AlreadyExists(env.name));
        }
        self.environments.push(env);
        Ok(())
    }

    /// Save configuration to a YAML file.
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<(), EnvironmentError> {
        let content = serde_yaml::to_string(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    /// Initialize a new environment with a bare git repository.
    /// Returns the created Environment.
    pub async fn initialize_new_environment(
        &mut self,
        name: &str,
    ) -> Result<Environment, EnvironmentError> {
        // Check if environment already exists
        if self.find(name).is_some() {
            return Err(EnvironmentError::AlreadyExists(name.to_string()));
        }

        // Create the environment directory
        let env_dir = self.new_environments_directory.join(name);
        tokio::fs::create_dir_all(&env_dir)
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        // Create bare repository
        let bare_path = env_dir.join("bare");
        tokio::fs::create_dir_all(&bare_path)
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        let output = Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .current_dir(&bare_path)
            .output()
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::GitInitError(stderr.to_string()));
        }

        // Clone, make initial commit, and push
        let temp_clone = env_dir.join("temp_clone");
        let output = Command::new("git")
            .args(["clone", bare_path.to_str().unwrap(), temp_clone.to_str().unwrap()])
            .output()
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::GitInitError(format!("Clone failed: {}", stderr)));
        }

        // Configure git user
        let _ = Command::new("git")
            .args(["config", "user.email", "slopcoder@example.com"])
            .current_dir(&temp_clone)
            .output()
            .await;

        let _ = Command::new("git")
            .args(["config", "user.name", "Slopcoder"])
            .current_dir(&temp_clone)
            .output()
            .await;

        // Create initial README
        tokio::fs::write(temp_clone.join("README.md"), format!("# {}\n\nCreated by Slopcoder.\n", name))
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        // Add and commit
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(&temp_clone)
            .output()
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::GitInitError(format!("Add failed: {}", stderr)));
        }

        let output = Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&temp_clone)
            .output()
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::GitInitError(format!("Commit failed: {}", stderr)));
        }

        // Push to origin
        let output = Command::new("git")
            .args(["push", "origin", "main"])
            .current_dir(&temp_clone)
            .output()
            .await
            .map_err(|e| EnvironmentError::GitInitError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(EnvironmentError::GitInitError(format!("Push failed: {}", stderr)));
        }

        // Remove temp clone
        let _ = tokio::fs::remove_dir_all(&temp_clone).await;

        // Create environment and add to config
        let env = Environment {
            name: name.to_string(),
            directory: env_dir,
        };

        self.environments.push(env.clone());

        Ok(env)
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

    /// Check if a branch exists in the bare repository.
    pub async fn branch_exists(&self, branch: &str) -> Result<bool, EnvironmentError> {
        let bare_path = self.bare_repo_path();
        let ref_name = format!("refs/heads/{}", branch);

        let output = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &ref_name])
            .current_dir(&bare_path)
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

    /// Create a new worktree for a new branch based on a base branch.
    pub async fn create_worktree_from_base(
        &self,
        base_branch: &str,
        feature_branch: &str,
    ) -> Result<PathBuf, EnvironmentError> {
        let worktree_path = self.worktree_path(feature_branch);

        if worktree_path.exists() {
            return Err(EnvironmentError::WorktreeExists(worktree_path));
        }

        if self.branch_exists(feature_branch).await? {
            return Err(EnvironmentError::BranchExists(feature_branch.to_string()));
        }

        let bare_path = self.bare_repo_path();
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                feature_branch,
                worktree_path.to_str().unwrap(),
                base_branch,
            ])
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
new_environments_directory: "/tmp/new-envs"
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
