use std::sync::Arc;

use axum::{routing::get, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

#[allow(dead_code)]
pub struct ServerConfig {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub anthropic_api_key: Option<String>,
    pub default_model: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://edgeclaw.db?mode=rwc".to_string()),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            default_model: std::env::var("CLAUDE_MODEL").ok(),
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Placeholder for the scheduler (M5.4)
pub struct Scheduler;

#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<ServerConfig>,
    pub scheduler: Arc<Scheduler>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .with_state(state)
}
