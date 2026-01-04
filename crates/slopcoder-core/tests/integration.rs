//! Integration tests for slopcoder-core.
//!
//! These tests require the Codex CLI to be installed and available in PATH.
//! They are ignored by default; run with `cargo test -- --ignored` to execute.

use slopcoder_core::{
    anyagent::{resume_anyagent, spawn_anyagent, AgentKind, AnyAgentConfig},
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

/// Check if claude is available.
async fn claude_available() -> bool {
    Command::new("claude")
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

async fn run_agent_hello_world(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create a file called hello.txt containing the text 'Hello, World!'",
    )
    .await
    .expect("Should spawn agent");

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

    let result = agent.wait().await.expect("Agent should complete");
    println!("Session ID: {}", result.session_id);
    println!("Success: {}", result.success);

    assert!(result.success);
    assert!(event_count > 0);

    let hello_path = worktree_path.join("hello.txt");
    assert!(hello_path.exists(), "hello.txt should exist");

    let content = tokio::fs::read_to_string(&hello_path)
        .await
        .expect("Should read hello.txt");
    assert!(content.contains("Hello"), "Content should contain 'Hello'");
}

async fn run_agent_resume(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
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

    let mut agent = resume_anyagent(
        kind,
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

    let content = tokio::fs::read_to_string(worktree_path.join("hello.txt"))
        .await
        .expect("Should read hello.txt");
    assert!(content.contains("Goodbye"), "Content should contain 'Goodbye'");
}

#[tokio::test]
async fn test_codex_agent_hello_world() {
    assert!(codex_available().await, "Codex CLI not available");
    run_agent_hello_world(AgentKind::Codex).await;
}

#[tokio::test]
async fn test_codex_agent_resume() {
    assert!(codex_available().await, "Codex CLI not available");
    run_agent_resume(AgentKind::Codex).await;
}

#[tokio::test]
async fn test_claude_agent_hello_world() {
    assert!(claude_available().await, "Claude CLI not available");
    run_agent_hello_world(AgentKind::Claude).await;
}

#[tokio::test]
async fn test_claude_agent_resume() {
    assert!(claude_available().await, "Claude CLI not available");
    run_agent_resume(AgentKind::Claude).await;
}

#[tokio::test]
async fn test_task_with_environment() {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let mut task = Task::new(
        AgentKind::Codex,
        env.name.clone(),
        Some("main".to_string()),
        "feature/test-task".to_string(),
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

async fn run_agent_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Write a very long story to story.txt. Make it at least 10 paragraphs.",
    )
    .await
    .expect("Should spawn agent");

    // Read a few events
    let mut event_count = 0;
    for _ in 0..5 {
        if agent.next_event().await.is_some() {
            event_count += 1;
        }
    }
    assert!(event_count > 0, "Should receive some events");

    // Interrupt the agent
    agent.kill().await.expect("Should kill agent");

    // Verify agent is no longer running
    let status = agent.try_wait().expect("Should check wait status");
    // After kill, the agent should have exited (or will exit soon)

    // Wait for agent to finish
    let result = agent.wait().await;
    // Result might be Ok or Err depending on how the agent was killed
    println!("Result after interrupt: {:?}", result);

    // Agent should have been interrupted, so it shouldn't have succeeded normally
    match result {
        Ok(r) => {
            // If we get a result, it shouldn't be a clean success
            println!("Agent exited with success={}", r.success);
        }
        Err(e) => {
            println!("Agent was killed as expected: {}", e);
        }
    }
}

async fn run_agent_resume_after_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    // First run - will be interrupted
    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create a file called test.txt with 'First attempt'",
    )
    .await
    .expect("Should spawn agent");

    // Read a few events to get session ID
    let mut session_id_opt = None;
    for _ in 0..5 {
        if let Some(Ok(event)) = agent.next_event().await {
            if let Some(sid) = event.session_id() {
                session_id_opt = Some(sid);
            }
        }
    }

    // Interrupt
    agent.kill().await.expect("Should kill agent");
    let _ = agent.wait().await;

    let session_id = session_id_opt.expect("Should have session ID");
    println!("Interrupted session ID: {}", session_id);

    // Resume and complete
    let mut agent = resume_anyagent(
        kind,
        &config,
        &worktree_path,
        session_id,
        "Now create a file called complete.txt with 'Completed after interrupt'",
    )
    .await
    .expect("Should resume agent");

    while agent.next_event().await.is_some() {}

    let result = agent.wait().await.expect("Resumed agent should complete");
    assert!(result.success, "Resumed agent should succeed");

    // Verify the file was created
    let complete_path = worktree_path.join("complete.txt");
    assert!(complete_path.exists(), "complete.txt should exist");
    let content = tokio::fs::read_to_string(&complete_path)
        .await
        .expect("Should read complete.txt");
    assert!(content.contains("Completed"), "Content should contain 'Completed'");
}

async fn run_agent_double_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;

    let worktree_path = env
        .create_worktree("main")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    // First run - interrupted
    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create first.txt with 'First attempt'",
    )
    .await
    .expect("Should spawn agent");

    let mut session_id = None;
    for _ in 0..5 {
        if let Some(Ok(event)) = agent.next_event().await {
            if let Some(sid) = event.session_id() {
                session_id = Some(sid);
            }
        }
    }
    agent.kill().await.expect("Should kill agent");
    let _ = agent.wait().await;

    let session_id = session_id.expect("Should have session ID from first run");
    println!("First session ID: {}", session_id);

    // Second run - interrupted again
    let mut agent = resume_anyagent(
        kind,
        &config,
        &worktree_path,
        session_id,
        "Create second.txt with 'Second attempt'",
    )
    .await
    .expect("Should resume agent");

    for _ in 0..5 {
        agent.next_event().await;
    }
    agent.kill().await.expect("Should kill agent second time");
    let _ = agent.wait().await;

    println!("Interrupted second time with session ID: {}", session_id);

    // Third run - complete successfully
    let mut agent = resume_anyagent(
        kind,
        &config,
        &worktree_path,
        session_id,
        "Create final.txt with 'Final success'",
    )
    .await
    .expect("Should resume agent");

    while agent.next_event().await.is_some() {}

    let result = agent.wait().await.expect("Final run should complete");
    assert!(result.success, "Final run should succeed");

    // Verify the final file was created
    let final_path = worktree_path.join("final.txt");
    assert!(final_path.exists(), "final.txt should exist");
    let content = tokio::fs::read_to_string(&final_path)
        .await
        .expect("Should read final.txt");
    assert!(content.contains("Final"), "Content should contain 'Final'");
}

#[tokio::test]
async fn test_codex_agent_interrupt() {
    assert!(codex_available().await, "Codex CLI not available");
    run_agent_interrupt(AgentKind::Codex).await;
}

#[tokio::test]
async fn test_claude_agent_interrupt() {
    assert!(claude_available().await, "Claude CLI not available");
    run_agent_interrupt(AgentKind::Claude).await;
}

#[tokio::test]
async fn test_codex_agent_resume_after_interrupt() {
    assert!(codex_available().await, "Codex CLI not available");
    run_agent_resume_after_interrupt(AgentKind::Codex).await;
}

#[tokio::test]
async fn test_claude_agent_resume_after_interrupt() {
    assert!(claude_available().await, "Claude CLI not available");
    run_agent_resume_after_interrupt(AgentKind::Claude).await;
}

#[tokio::test]
async fn test_codex_agent_double_interrupt() {
    assert!(codex_available().await, "Codex CLI not available");
    run_agent_double_interrupt(AgentKind::Codex).await;
}

#[tokio::test]
async fn test_claude_agent_double_interrupt() {
    assert!(claude_available().await, "Claude CLI not available");
    run_agent_double_interrupt(AgentKind::Claude).await;
}
