mod routes;
mod state;

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use warp::Filter;

use state::AppState;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("slopcoder=info".parse().unwrap()))
        .init();

    // Parse CLI args
    let mut args = std::env::args().skip(1);
    let mut config_path: Option<PathBuf> = None;
    let mut addr_arg: Option<String> = None;
    let mut static_dir_arg: Option<PathBuf> = None;
    let mut branch_model = "claude-haiku-4-5".to_string();
    let mut password_prompt = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" | "--bind" => {
                addr_arg = args.next();
            }
            "--static-dir" | "--assets" => {
                static_dir_arg = args.next().map(PathBuf::from);
            }
            "--branch-model" => {
                if let Some(value) = args.next() {
                    branch_model = value;
                }
            }
            "--password-prompt" => {
                password_prompt = true;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: slopcoder-server [config.yaml] [--addr HOST:PORT] [--static-dir PATH] [--branch-model MODEL] [--password-prompt]\n\
Defaults: config=environments.yaml, addr=127.0.0.1:8080, static-dir=frontend/dist, branch-model=claude-haiku-4-5"
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
    let auth_password = if password_prompt {
        print!("Enter password: ");
        io::stdout().flush().expect("Failed to flush stdout");
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read password");
        let trimmed = input.trim_end_matches(&['\r', '\n'][..]).to_string();
        if trimmed.is_empty() {
            None
        } else {
            println!("Password set to: {}", trimmed);
            Some(trimmed)
        }
    } else {
        None
    };

    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .and_then(|v| if v.trim().is_empty() { None } else { Some(v) });
    if api_key.is_none() {
        tracing::warn!("OPENAI_API_KEY is not set; automatic branch naming will be unavailable.");
    }
    let api_base = std::env::var("OPENAI_API_BASE")
        .ok()
        .and_then(|v| if v.trim().is_empty() { None } else { Some(v) });
    if api_base.is_none() {
        tracing::warn!("OPENAI_API_BASE is not set; using the default OpenAI base URL.");
    }

    // Check if config exists
    if !config_path.exists() {
        tracing::error!("Config file not found: {}", config_path.display());
        tracing::info!(
            "Usage: slopcoder-server [config.yaml] [--addr HOST:PORT] [--static-dir PATH] [--branch-model MODEL] [--password-prompt]"
        );
        tracing::info!("Example config:");
        tracing::info!(
            r#"
environments:
  - name: "my-project"
    directory: "/path/to/project"
"#
        );
        std::process::exit(1);
    }

    // Load state
    let state = match AppState::new(config_path.clone(), branch_model, auth_password).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Startup checks failed: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!("Loaded config from {}", config_path.display());

    // Log environment info
    let config = state.get_config().await;
    for env in &config.environments {
        tracing::info!(
            "Environment '{}' at {} (tasks.yaml: {})",
            env.name,
            env.directory.display(),
            env.directory.join("tasks.yaml").display()
        );
    }

    // Log task count
    let tasks = state.list_tasks().await;
    tracing::info!("Loaded {} tasks from disk", tasks.len());

    // Build API routes
    let api_routes = routes::routes(state);

    // Add CORS for development
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allow_headers(vec!["Content-Type", "X-Slopcoder-Password"]);

    let api_routes = api_routes.with(cors);

    // Static file serving for frontend
    let static_dir = static_dir_arg
        .or_else(|| std::env::var("SLOPCODER_STATIC_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("frontend/dist"));

    let static_files = warp::fs::dir(static_dir.clone());

    // Serve index.html for SPA routes (fallback for client-side routing)
    let index_html = warp::fs::file(static_dir.join("index.html"));
    let spa_fallback = warp::any()
        .and(warp::get())
        .and(index_html);

    // Combine: API routes first, then static files, then SPA fallback
    let routes = api_routes
        .or(static_files)
        .or(spa_fallback);

    // Get address from args/env or use default (127.0.0.1:8080)
    let addr: SocketAddr = addr_arg
        .or_else(|| std::env::var("SLOPCODER_ADDR").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| ([127, 0, 0, 1], 8080).into());

    tracing::info!("Starting server at http://{}", addr);

    warp::serve(routes).run(addr).await;
}
