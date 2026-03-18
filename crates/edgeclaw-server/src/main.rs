mod server;

use std::sync::Arc;

use anyhow::Result;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use server::{build_router, AppState, Scheduler, ServerConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    dotenvy::dotenv().ok();

    let config = ServerConfig::from_env();
    let bind_addr = config.bind_addr();

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    let state = AppState {
        db: pool,
        config: Arc::new(config),
        scheduler: Arc::new(Scheduler),
    };

    let app = build_router(state);
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("listening on {}", bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
