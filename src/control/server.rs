//! The control endpoint server: a tiny HTTP/1.1 cleartext service that
//! publishes the [`ServerManifest`] as JSON. It carries only addresses
//! and version metadata — no payload, no secrets — so it is
//! deliberately plain HTTP.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::get};
use eyre::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::control::manifest::ServerManifest;

/// Configuration for the control endpoint.
pub struct ControlServerConfig {
    /// Address (bind IP + the user-chosen control port) to listen on.
    pub bind_addr: SocketAddr,
    /// The manifest to publish. Built after every test listener has
    /// bound so the advertised ports are real.
    pub manifest: Arc<ServerManifest>,
}

/// Run the control endpoint, gracefully shutting down when `cancel`
/// fires. Serves the manifest at `GET /` and `GET /manifest`.
pub async fn run_control_server(
    config: ControlServerConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let manifest = config.manifest;
    let app = Router::new()
        .route("/", get(manifest_handler))
        .route("/manifest", get(manifest_handler))
        .route("/health", get(health_handler))
        .with_state(manifest);

    tracing::info!("Control endpoint listening on {}", config.bind_addr);
    // The control port is the one fixed, user-chosen port; SO_REUSEADDR
    // (set by the helper on Unix) lets a restarted server rebind it
    // while old connections sit in TIME_WAIT.
    let listener = crate::utils::net::bind_tcp_listener(config.bind_addr, None)
        .wrap_err_with(|| format!("Failed to bind control endpoint on {}", config.bind_addr))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
            tracing::info!("Control endpoint received shutdown signal, draining...");
        })
        .await
        .wrap_err("Control endpoint server error")?;

    Ok(())
}

async fn manifest_handler(State(manifest): State<Arc<ServerManifest>>) -> Json<ServerManifest> {
    Json((*manifest).clone())
}

async fn health_handler() -> &'static str {
    "ok"
}
