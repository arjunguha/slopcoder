mod routes;
mod state;

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

    // Get config path from args or use default
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("environments.yaml"));

    // Check if config exists
    if !config_path.exists() {
        tracing::error!("Config file not found: {}", config_path.display());
        tracing::info!("Usage: slopcoder-server <config.yaml>");
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
    let state = match AppState::new(config_path.clone()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!("Loaded config from {}", config_path.display());

    // Build API routes
    let api_routes = routes::routes(state);

    // Add CORS for development
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allow_headers(vec!["Content-Type"]);

    let api_routes = api_routes.with(cors);

    // Static file serving for frontend
    let static_dir = std::env::var("SLOPCODER_STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("frontend/dist"));

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

    // Get address from env or use default (0.0.0.0:3000)
    let addr: SocketAddr = std::env::var("SLOPCODER_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| ([0, 0, 0, 0], 3000).into());

    tracing::info!("Starting server at http://{}", addr);

    warp::serve(routes).run(addr).await;
}
