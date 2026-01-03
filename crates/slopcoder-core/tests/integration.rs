//! Integration tests for slopcoder-core.
//!
//! These tests require the Codex CLI to be installed and available in PATH.
//! They are ignored by default; run with `cargo test -- --ignored` to execute.

use slopcoder_core::{
    agent::{Agent, AgentConfig},
    environment::{Environment, EnvironmentConfig},
    task::{Task, TaskStatus},
};
use tempfile::TempDir;
use tokio::process::Command;

/// Check if codex is available.
async fn codex_available() -> bool {
    Command::new("codex")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Set up a test environment with a bare git repo.
async fn setup_test_env() -> (TempDir, Environment) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path().to_path_buf();

    // Create bare repo
    let bare_path = base_path.join("bare");
    tokio::fs::create_dir_all(&bare_path)
        .await
        .expect("Failed to create bare dir");

    // Initialize bare repo
    let status = Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&bare_path)
        .status()
        .await
        .expect("Failed to run git init");
    assert!(status.success());

    // Create a temporary clone to make initial commit
    let clone_path = base_path.join("temp_clone");
    let status = Command::new("git")
        .args(["clone", bare_path.to_str().unwrap(), clone_path.to_str().unwrap()])
        .status()
        .await
        .expect("Failed to clone");
    assert!(status.success());

    // Configure git and create initial commit
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&clone_path)
        .status()
        .await
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&clone_path)
        .status()
        .await
        .unwrap();

    tokio::fs::write(clone_path.join("README.md"), "# Test Project\n")
        .await
        .expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&clone_path)
        .status()
        .await
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&clone_path)
        .status()
        .await
        .unwrap();

    Command::new("git")
        .args(["push", "origin", "HEAD:main"])
        .current_dir(&clone_path)
        .status()
        .await
        .unwrap();

    // Clean up clone
    tokio::fs::remove_dir_all(&clone_path).await.ok();

    let env = Environment {
        name: "test-env".to_string(),
        directory: base_path,
    };

    (temp_dir, env)
}

#[tokio::test]
async fn test_environment_validation() {
    let (_temp_dir, env) = setup_test_env().await;

    // Should validate successfully
    env.validate().await.expect("Environment should be valid");

    // Should list branches
    let branches = env.list_branches().await.expect("Should list branches");
    assert!(branches.contains(&"main".to_string()));
}

#[tokio::test]
async fn test_worktree_creation() {
    let (_temp_dir, env) = setup_test_env().await;

    // Create worktree
    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    assert!(worktree_path.exists());
    assert!(worktree_path.join("README.md").exists());

    // Should fail if worktree already exists
    let result = env.create_worktree("main").await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "Requires Codex CLI and API key"]
async fn test_agent_hello_world() {
    if !codex_available().await {
        eprintln!("Codex not available, skipping test");
        return;
    }

    let (_temp_dir, env) = setup_test_env().await;

    // Create worktree
    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    // Create agent config
    let config = AgentConfig::default();

    // Spawn agent
    let mut agent = Agent::spawn(
        &config,
        &worktree_path,
        "Create a file called hello.txt containing the text 'Hello, World!'",
    )
    .await
    .expect("Should spawn agent");

    // Collect events
    let mut event_count = 0;
    while let Some(result) = agent.next_event().await {
        match result {
            Ok(event) => {
                event_count += 1;
                println!("Event: {:?}", event);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    // Wait for completion
    let result = agent.wait().await.expect("Agent should complete");
    println!("Session ID: {}", result.session_id);
    println!("Success: {}", result.success);

    assert!(result.success);
    assert!(event_count > 0);

    // Check that the file was created
    let hello_path = worktree_path.join("hello.txt");
    assert!(hello_path.exists(), "hello.txt should exist");

    let content = tokio::fs::read_to_string(&hello_path)
        .await
        .expect("Should read hello.txt");
    assert!(
        content.contains("Hello"),
        "Content should contain 'Hello'"
    );
}

#[tokio::test]
#[ignore = "Requires Codex CLI and API key"]
async fn test_agent_resume() {
    if !codex_available().await {
        eprintln!("Codex not available, skipping test");
        return;
    }

    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AgentConfig::default();

    // First run: create hello.txt
    let mut agent = Agent::spawn(
        &config,
        &worktree_path,
        "Create a file called hello.txt containing 'Hello, World!'",
    )
    .await
    .expect("Should spawn agent");

    while agent.next_event().await.is_some() {}

    let result1 = agent.wait().await.expect("First run should complete");
    assert!(result1.success);

    let session_id = result1.session_id;
    println!("First session ID: {}", session_id);

    // Second run: modify the file using resume
    let mut agent = Agent::resume(
        &config,
        &worktree_path,
        session_id,
        "Change hello.txt to say 'Goodbye, World!' instead",
    )
    .await
    .expect("Should resume agent");

    while agent.next_event().await.is_some() {}

    let result2 = agent.wait().await.expect("Second run should complete");
    assert!(result2.success);

    // Verify the file was modified
    let content = tokio::fs::read_to_string(worktree_path.join("hello.txt"))
        .await
        .expect("Should read hello.txt");
    assert!(
        content.contains("Goodbye"),
        "Content should contain 'Goodbye'"
    );
}

#[tokio::test]
async fn test_task_with_environment() {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let mut task = Task::new(
        "Test Task".to_string(),
        env.name.clone(),
        "main".to_string(),
        worktree_path.clone(),
    );

    assert_eq!(task.status, TaskStatus::Pending);

    task.start_run("Hello world".to_string());
    assert_eq!(task.status, TaskStatus::Running);

    // Simulate completion
    task.complete_run(true);
    assert_eq!(task.status, TaskStatus::Completed);
}

#[tokio::test]
async fn test_config_loading() {
    let yaml = r#"
environments:
  - name: "test-project"
    directory: "/tmp/test-project"
"#;

    let config = EnvironmentConfig::from_yaml(yaml).expect("Should parse config");
    assert_eq!(config.environments.len(), 1);
    assert_eq!(config.environments[0].name, "test-project");
}
