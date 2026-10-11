use axum::Router;
use std::path::PathBuf;
use topology_checker_web::{AppState, api};
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let assets = std::env::var_os("TOPOLOGY_WEB_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let beside = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("web")));
            beside
                .filter(|p| p.join("index.html").exists())
                .unwrap_or_else(|| PathBuf::from("web/dist"))
        });
    if !assets.join("index.html").exists() {
        eprintln!(
            "Frontend assets missing at {}. Run npm ci and npm run build in web, or use the Vite development server.",
            assets.display()
        );
    }
    let frontend =
        ServeDir::new(&assets).not_found_service(ServeFile::new(assets.join("index.html")));
    let app = Router::new()
        .nest("/api", api(AppState::default()))
        .fallback_service(frontend);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    println!("Topology checker: http://127.0.0.1:8080");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
