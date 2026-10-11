use axum::Router;
use clap::Parser;
use std::time::Duration;
use topology_checker_web::{AppState, IDLE_TIMEOUT, api, assets};

/// How often idle workspaces and their temporary files are discarded.
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Serve the local topology checker on the loopback interface.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Port on 127.0.0.1; 0 selects any free port.
    #[arg(long, env = "TOPOLOGY_WEB_PORT", default_value_t = 8080)]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port)).await?;
    let port = listener.local_addr()?.port();
    let state = AppState::new(port);
    let sweeper = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            sweeper.evict_idle(IDLE_TIMEOUT);
        }
    });
    let app = Router::new()
        .nest("/api", api(state))
        .fallback(assets::serve);
    println!("Topology checker: http://127.0.0.1:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
