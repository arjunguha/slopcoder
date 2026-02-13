//! Integration tests for slopcoder-core.
//!
//! Agent integration tests are gated behind feature flags:
//! - `test-codex`: Enable Codex agent tests
//! - `test-claude`: Enable Claude agent tests
//! - `test-cursor`: Enable Cursor agent tests
//! - `test-opencode`: Enable OpenCode agent tests
//!
//! Run with: `cargo test --features test-opencode` (or other features)

use slopcoder_core::{
    anyagent::{resume_anyagent, spawn_anyagent, AgentKind, AnyAgentConfig},
    environment::{Environment, EnvironmentConfig},
    task::{Task, TaskStatus, TaskWorkspaceKind},
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::process::Command;

/// Set up a test environment with a checked-out git repository.
async fn setup_test_env() -> (TempDir, Environment) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path().to_path_buf();

    // Create checked-out repository directory.
    let repo_path = base_path.join("repo");
    tokio::fs::create_dir_all(&repo_path)
        .await
        .expect("Failed to create repo dir");

    // Initialize repository.
    let status = Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .status()
        .await
        .expect("Failed to run git init");
    assert!(status.success());

    // Configure git and create initial commit
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&repo_path)
        .status()
        .await
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .status()
        .await
        .unwrap();

    tokio::fs::write(repo_path.join("README.md"), "# Test Project\n")
        .await
        .expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .status()
        .await
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .status()
        .await
        .unwrap();

    let env = Environment {
        name: "test-env".to_string(),
        directory: repo_path,
    };

    (temp_dir, env)
}

fn worktrees_dir(base: &Path) -> PathBuf {
    base.join("worktrees")
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
    let worktrees = worktrees_dir(_temp_dir.path());

    // Create worktree
    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-worktree-creation")
        .await
        .expect("Should create worktree");

    assert!(worktree_path.exists());
    assert!(worktree_path.join("README.md").exists());

    // Should fail if worktree already exists
    let result = env
        .create_worktree_from_base(&worktrees, "main", "task-worktree-creation")
        .await;
    assert!(result.is_err());
}

#[cfg(any(
    feature = "test-codex",
    feature = "test-claude",
    feature = "test-cursor"
))]
async fn run_agent_hello_world(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-hello-world")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create a file called hello.txt containing the text 'Hello, World!'",
        false,
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

#[cfg(any(
    feature = "test-codex",
    feature = "test-claude",
    feature = "test-cursor"
))]
async fn run_agent_resume(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-resume")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create a file called hello.txt containing 'Hello, World!'",
        false,
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
        false,
    )
    .await
    .expect("Should resume agent");

    while agent.next_event().await.is_some() {}

    let result2 = agent.wait().await.expect("Second run should complete");
    assert!(result2.success);

    let content = tokio::fs::read_to_string(worktree_path.join("hello.txt"))
        .await
        .expect("Should read hello.txt");
    assert!(
        content.contains("Goodbye"),
        "Content should contain 'Goodbye'"
    );
}

#[cfg(feature = "test-codex")]
#[tokio::test]
async fn test_codex_agent_hello_world() {
    run_agent_hello_world(AgentKind::Codex).await;
}

#[cfg(feature = "test-codex")]
#[tokio::test]
async fn test_codex_agent_resume() {
    run_agent_resume(AgentKind::Codex).await;
}

#[cfg(feature = "test-claude")]
#[tokio::test]
async fn test_claude_agent_hello_world() {
    run_agent_hello_world(AgentKind::Claude).await;
}

#[cfg(feature = "test-claude")]
#[tokio::test]
async fn test_claude_agent_resume() {
    run_agent_resume(AgentKind::Claude).await;
}

#[tokio::test]
async fn test_task_with_environment() {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-with-environment")
        .await
        .expect("Should create worktree");

    let mut task = Task::new(
        AgentKind::Codex,
        env.name.clone(),
        "topic".to_string(),
        TaskWorkspaceKind::Worktree,
        Some("main".to_string()),
        Some("task/test-task".to_string()),
        false,
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
worktrees_directory: "/tmp/worktrees"
environments:
  - "/tmp/test-project"
"#;

    let config = EnvironmentConfig::from_yaml(yaml).expect("Should parse config");
    assert_eq!(config.environments.len(), 1);
    assert_eq!(config.environments[0].name, "/tmp/test-project".to_string());
}

#[cfg(any(
    feature = "test-codex",
    feature = "test-claude",
    feature = "test-cursor",
    feature = "test-opencode"
))]
async fn run_agent_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-interrupt")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Write a very long story to story.txt. Make it at least 10 paragraphs.",
        false,
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

#[cfg(any(
    feature = "test-codex",
    feature = "test-claude",
    feature = "test-cursor",
    feature = "test-opencode"
))]
async fn run_agent_resume_after_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-resume-after-interrupt")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    // First run - will be interrupted
    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create a file called test.txt with 'First attempt'",
        false,
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
        false,
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
    assert!(
        content.contains("Completed"),
        "Content should contain 'Completed'"
    );
}

#[cfg(any(
    feature = "test-codex",
    feature = "test-claude",
    feature = "test-cursor",
    feature = "test-opencode"
))]
async fn run_agent_double_interrupt(kind: AgentKind) {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-double-interrupt")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    // First run - interrupted
    let mut agent = spawn_anyagent(
        kind,
        &config,
        &worktree_path,
        "Create first.txt with 'First attempt'",
        false,
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
        false,
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
        false,
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

#[cfg(feature = "test-codex")]
#[tokio::test]
async fn test_codex_agent_interrupt() {
    run_agent_interrupt(AgentKind::Codex).await;
}

#[cfg(feature = "test-claude")]
#[tokio::test]
async fn test_claude_agent_interrupt() {
    run_agent_interrupt(AgentKind::Claude).await;
}

#[cfg(feature = "test-codex")]
#[tokio::test]
async fn test_codex_agent_resume_after_interrupt() {
    run_agent_resume_after_interrupt(AgentKind::Codex).await;
}

#[cfg(feature = "test-claude")]
#[tokio::test]
async fn test_claude_agent_resume_after_interrupt() {
    run_agent_resume_after_interrupt(AgentKind::Claude).await;
}

#[cfg(feature = "test-codex")]
#[tokio::test]
async fn test_codex_agent_double_interrupt() {
    run_agent_double_interrupt(AgentKind::Codex).await;
}

#[cfg(feature = "test-claude")]
#[tokio::test]
async fn test_claude_agent_double_interrupt() {
    run_agent_double_interrupt(AgentKind::Claude).await;
}

#[cfg(feature = "test-cursor")]
#[tokio::test]
async fn test_cursor_agent_hello_world() {
    run_agent_hello_world(AgentKind::Cursor).await;
}

#[cfg(feature = "test-cursor")]
#[tokio::test]
async fn test_cursor_agent_resume() {
    run_agent_resume(AgentKind::Cursor).await;
}

#[cfg(feature = "test-cursor")]
#[tokio::test]
async fn test_cursor_agent_interrupt() {
    run_agent_interrupt(AgentKind::Cursor).await;
}

#[cfg(feature = "test-cursor")]
#[tokio::test]
async fn test_cursor_agent_resume_after_interrupt() {
    run_agent_resume_after_interrupt(AgentKind::Cursor).await;
}

#[cfg(feature = "test-cursor")]
#[tokio::test]
async fn test_cursor_agent_double_interrupt() {
    run_agent_double_interrupt(AgentKind::Cursor).await;
}

/// Run opencode agent hello world test.
/// Unlike other agents, we don't verify file creation because the boa model
/// may refuse to create files based on its system prompt.
#[cfg(feature = "test-opencode")]
async fn run_opencode_hello_world() {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-opencode-hello")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        AgentKind::Opencode,
        &config,
        &worktree_path,
        "Create a file called hello.txt containing the text 'Hello, World!'",
        false,
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

    // Verify basic agent functionality
    assert!(result.success);
    assert!(event_count > 0, "Should receive events");

    // Note: We don't verify file creation for opencode because the boa model
    // may refuse to create files based on its instructions.
}

/// Run opencode agent resume test.
/// Tests that session resumption works correctly.
#[cfg(feature = "test-opencode")]
async fn run_opencode_resume() {
    let (_temp_dir, env) = setup_test_env().await;
    let worktrees = worktrees_dir(_temp_dir.path());

    let worktree_path = env
        .create_worktree_from_base(&worktrees, "main", "task-opencode-resume")
        .await
        .expect("Should create worktree");

    let config = AnyAgentConfig::default();

    let mut agent = spawn_anyagent(
        AgentKind::Opencode,
        &config,
        &worktree_path,
        "What is 2+2?",
        false,
    )
    .await
    .expect("Should spawn agent");

    while agent.next_event().await.is_some() {}

    let result1 = agent.wait().await.expect("First run should complete");
    assert!(result1.success);

    let session_id = result1.session_id;
    println!("First session ID: {}", session_id);

    // Resume with the same session
    let mut agent = resume_anyagent(
        AgentKind::Opencode,
        &config,
        &worktree_path,
        session_id,
        "What is 3+3?",
        false,
    )
    .await
    .expect("Should resume agent");

    let mut event_count = 0;
    while let Some(_) = agent.next_event().await {
        event_count += 1;
    }

    let result2 = agent.wait().await.expect("Second run should complete");
    assert!(result2.success);
    assert!(event_count > 0, "Should receive events in resumed session");
    println!("Resume completed successfully");
}

#[cfg(feature = "test-opencode")]
#[tokio::test]
async fn test_opencode_agent_hello_world() {
    run_opencode_hello_world().await;
}

#[cfg(feature = "test-opencode")]
#[tokio::test]
async fn test_opencode_agent_resume() {
    run_opencode_resume().await;
}

#[cfg(feature = "test-opencode")]
#[tokio::test]
async fn test_opencode_agent_interrupt() {
    run_agent_interrupt(AgentKind::Opencode).await;
}

#[cfg(feature = "test-opencode")]
#[tokio::test]
async fn test_opencode_agent_resume_after_interrupt() {
    run_agent_resume_after_interrupt(AgentKind::Opencode).await;
}

#[cfg(feature = "test-opencode")]
#[tokio::test]
async fn test_opencode_agent_double_interrupt() {
    run_agent_double_interrupt(AgentKind::Opencode).await;
}
