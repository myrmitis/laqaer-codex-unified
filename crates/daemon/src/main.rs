use codex_unifiedd::{AppConfig, app};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("codex_unifiedd=info")),
        )
        .init();

    let capability = std::env::var("CODEX_UNIFIED_CAPABILITY")
        .expect("CODEX_UNIFIED_CAPABILITY must be set by the service supervisor");

    let app = app(AppConfig { capability });
    let address = SocketAddr::from(([127, 0, 0, 1], 4317));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind codex-unifiedd");

    tracing::info!(%address, "codex-unifiedd listening");
    axum::serve(listener, app)
        .await
        .expect("serve codex-unifiedd");
}
