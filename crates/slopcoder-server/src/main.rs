mod routes;
mod state;

use std::io::{self, Write};
use std::net::SocketAddr;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use uuid::Uuid;
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
    let mut addr_arg: Option<String> = None;
    let mut static_dir_arg: Option<std::path::PathBuf> = None;
    let mut ui_password_prompt = false;
    let mut agent_password_prompt = false;
    let mut no_password = false;
    let mut explicit_ui_password: Option<String> = None;
    let mut explicit_agent_password: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" | "--bind" => {
                addr_arg = args.next();
            }
            "--static-dir" | "--assets" => {
                static_dir_arg = args.next().map(std::path::PathBuf::from);
            }
            "--password-prompt" => {
                ui_password_prompt = true;
            }
            "--password" => {
                explicit_ui_password = args.next();
            }
            "--agent-password-prompt" => {
                agent_password_prompt = true;
            }
            "--agent-password" => {
                explicit_agent_password = args.next();
            }
            "--no-password" => {
                no_password = true;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: slopcoder-server [--addr HOST:PORT] [--static-dir PATH] [--password VALUE|--password-prompt|--no-password] [--agent-password VALUE|--agent-password-prompt]\n\
Defaults: addr=127.0.0.1:8080, static-dir=frontend/dist, UI auth disabled, agent auth enabled with generated startup password"
                );
                return;
            }
            _ => {}
        }
    }

    let ui_auth_password = if no_password {
        tracing::warn!("UI authentication disabled (--no-password).");
        None
    } else if let Some(password) = explicit_ui_password {
        println!("UI password: {}", password);
        Some(password)
    } else if ui_password_prompt {
        print!("Enter UI password: ");
        io::stdout().flush().expect("Failed to flush stdout");
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read password");
        let trimmed = input.trim_end_matches(&['\r', '\n'][..]).to_string();
        if trimmed.is_empty() {
            None
        } else {
            println!("UI password: {}", trimmed);
            Some(trimmed)
        }
    } else {
        None
    };

    let agent_auth_password = if let Some(password) = explicit_agent_password {
        password
    } else if agent_password_prompt {
        print!("Enter slopagent connection password: ");
        io::stdout().flush().expect("Failed to flush stdout");
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read password");
        let trimmed = input.trim_end_matches(&['\r', '\n'][..]).to_string();
        if trimmed.is_empty() {
            tracing::error!("Agent password cannot be empty");
            std::process::exit(1);
        }
        trimmed
    } else {
        Uuid::new_v4().simple().to_string()[..16].to_string()
    };
    println!("Slopagent password: {}", agent_auth_password);

    let state = AppState::new(ui_auth_password, agent_auth_password);

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
        .or_else(|| {
            std::env::var("SLOPCODER_STATIC_DIR")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| std::path::PathBuf::from("frontend/dist"));

    let static_files = warp::fs::dir(static_dir.clone());

    // Serve index.html for SPA routes (fallback for client-side routing)
    let index_html = warp::fs::file(static_dir.join("index.html"));
    let spa_fallback = warp::any().and(warp::get()).and(index_html);

    // Combine: API routes first, then static files, then SPA fallback
    let routes = api_routes.or(static_files).or(spa_fallback);

    // Get address from args/env or use default (127.0.0.1:8080)
    let addr: SocketAddr = addr_arg
        .or_else(|| std::env::var("SLOPCODER_ADDR").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| ([127, 0, 0, 1], 8080).into());

    tracing::info!("Starting server at http://{}", addr);

    warp::serve(routes).run(addr).await;
}
