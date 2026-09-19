use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

async fn health() -> Json<Value> {
    Json(json!({
        "service": "codex-unifiedd",
        "status": "ok",
        "protocol": 1
    }))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("codex_unifiedd=info")),
        )
        .init();

    let app = Router::new().route("/healthz", get(health));
    let address = SocketAddr::from(([127, 0, 0, 1], 4317));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind codex-unifiedd");

    tracing::info!(%address, "codex-unifiedd listening");
    axum::serve(listener, app)
        .await
        .expect("serve codex-unifiedd");
}
