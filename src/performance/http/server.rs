use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use eyre::Result;
use futures::StreamExt as _;
use futures::stream;
use http_body_util::BodyExt;
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::{net::SocketAddr, path::PathBuf, sync::Once};
use tokio_util::io::ReaderStream;
use tower_http::cors::{Any, CorsLayer};
use tracing::debug;

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    });
}

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    /// Bind address
    pub bind_addr: SocketAddr,
    /// Enable cors. Usually should be true.
    pub enable_cors: bool,
    /// Max upload size in bytes
    pub max_upload_size: usize,
}

#[derive(Debug, Clone)]
pub struct HttpsServerConfig {
    /// Bind address
    pub bind_addr: SocketAddr,
    /// Enable cors. Usually should be true.
    pub enable_cors: bool,
    /// Max upload size in bytes
    pub max_upload_size: usize,

    /// Path to the TLS certificate
    pub cert_path: PathBuf,
    /// Path to the TLS private key
    pub key_path: PathBuf,
}

/// Runs the HTTP server.
pub async fn run_http_server(config: HttpServerConfig) -> Result<()> {
    let app = create_router(config.enable_cors, config.max_upload_size);

    tracing::info!("HTTP server listening on {}", config.bind_addr);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Runs the HTTPS server.
pub async fn run_https_server(config: HttpsServerConfig) -> Result<()> {
    // Ensure crypto provider is initialized before using TLS
    ensure_crypto_provider();

    let app = create_router(config.enable_cors, config.max_upload_size);
    let tls_config = RustlsConfig::from_pem_file(config.cert_path, config.key_path).await?;

    tracing::info!("HTTPS server listening on {}", config.bind_addr);
    axum_server::bind_rustls(config.bind_addr, tls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn create_router(enable_cors: bool, max_upload_size: usize) -> Router {
    let mut router = Router::new()
        .route("/download", get(download_handler))
        .route("/upload", post(upload_handler))
        .route("/latency", get(latency_handler).head(latency_handler))
        .route("/info", get(info_handler))
        .route("/health", get(health_handler))
        .layer(DefaultBodyLimit::max(max_upload_size));

    if enable_cors {
        router = router.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::HEAD])
                .allow_headers(Any),
        );
    }

    router = router.layer(tower_http::trace::TraceLayer::new_for_http().on_response(
        |response: &Response<_>, latency: std::time::Duration, _span: &tracing::Span| {
            debug!(
                status = ?response.status(),
                latency = ?latency,
                "HTTP response"
            );
        },
    ));

    router
}

#[derive(Deserialize)]
struct DownloadQuery {
    size: usize,
}

async fn download_handler(Query(query): Query<DownloadQuery>) -> impl IntoResponse {
    // Create a stream that yields chunks of data
    let chunk_size = 8192; // 8KB chunks
    let total_size = query.size;
    let chunks = total_size.div_ceil(chunk_size); // Round up division

    let stream = stream::iter(0..chunks).map(move |i| {
        let remaining = total_size - (i * chunk_size);
        let current_chunk_size = chunk_size.min(remaining);
        Ok::<_, std::io::Error>(vec![0u8; current_chunk_size])
    });

    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, query.size.to_string())
        .body(body)
        .unwrap()
}

async fn upload_handler(body: Body) -> impl IntoResponse {
    let mut body_reader = body.into_data_stream();
    let mut total_bytes = 0;
    while let Some(chunk) = body_reader.next().await {
        total_bytes += chunk.unwrap().len();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bytes_received": total_bytes })),
    )
}

async fn latency_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[derive(Serialize)]
struct ServerInfo {
    server_name: String,
    version: String,
    available_endpoints: Vec<&'static str>,
}

async fn info_handler() -> impl IntoResponse {
    let info = ServerInfo {
        server_name: "Rust Hyper/Axum Server".to_string(),
        version: "1.0.0".to_string(),
        available_endpoints: vec!["/download", "/upload", "/latency", "/info", "/health"],
    };
    (StatusCode::OK, Json(info))
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}
