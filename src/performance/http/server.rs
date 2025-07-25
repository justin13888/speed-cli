use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use bytes::Bytes;
use eyre::Result;
use futures::StreamExt as _;
use futures::stream;
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock as SyncLazy;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, sync::Once};
use tower_http::cors::{Any, CorsLayer};

use crate::utils::tls::get_self_signed_cert;

use crate::constants::DEFAULT_CHUNK_SIZE;

/// Static buffer for download operations to avoid allocations
static ZERO_BUFFER: SyncLazy<Arc<Bytes>> = SyncLazy::new(|| {
    // 64KB zero buffer - large enough to avoid frequent copying but small enough for L1/L2 cache
    Arc::new(Bytes::from(vec![0u8; 1024 * 1024 * 1024])) // 1GB buffer
});

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

    /// TLS config
    /// If not provided, a self-signed certificate will be generated
    pub tls_config: Option<TlsConfig>,
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the TLS certificate (PEM format)
    pub cert_path: PathBuf,
    /// Path to the TLS private key (PEM format)
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
    let tls_config = match config.tls_config {
        Some(tls_config) => {
            RustlsConfig::from_pem_file(tls_config.cert_path, tls_config.key_path).await?
        }
        None => get_self_signed_cert().await?,
    };

    tracing::info!("HTTPS server listening on {}", config.bind_addr);

    // For axum_server, we bind and serve directly
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

    // // Use sampling-based tracing for high-throughput scenarios
    // router = router.layer(
    //     tower_http::trace::TraceLayer::new_for_http()
    //         .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::DEBUG))
    //         .on_response(
    //             |response: &Response<_>, latency: std::time::Duration, _span: &tracing::Span| {
    //                 // Only log a subset of responses to reduce overhead during high load
    //                 if rand::random::<f32>() < 0.1 {
    //                     // 10% sampling rate
    //                     debug!(
    //                         status = ?response.status(),
    //                         latency = ?latency,
    //                         "HTTP response"
    //                     );
    //                 }
    //             },
    //         ),
    // );

    router
}

#[derive(Deserialize)]
struct DownloadQuery {
    size: usize,
    #[serde(default = "default_chunk_size")]
    chunk_size: usize,
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

async fn download_handler(Query(query): Query<DownloadQuery>) -> impl IntoResponse {
    // Use the static buffer to avoid allocations
    let total_size = query.size;
    let chunk_size = query.chunk_size;
    let chunks = total_size.div_ceil(chunk_size); // Round up division

    let buffer_ref = Arc::clone(&ZERO_BUFFER);

    // TODO: Check how slow this is vv
    let stream = stream::iter(0..chunks).enumerate().map(move |(i, _)| {
        let bytes_sent = i * chunk_size;
        let remaining_bytes = total_size.saturating_sub(bytes_sent);
        let current_chunk_size = chunk_size.min(remaining_bytes);

        // If the chunk size is larger than our buffer, we need to repeat the buffer
        if current_chunk_size <= buffer_ref.len() {
            let bytes = buffer_ref.clone().slice(0..current_chunk_size);
            Ok::<_, std::io::Error>(bytes)
        } else {
            // Create a larger chunk by repeating the buffer
            let mut chunk_data = Vec::with_capacity(current_chunk_size);
            let mut bytes_written = 0;
            while bytes_written < current_chunk_size {
                let bytes_to_copy = (current_chunk_size - bytes_written).min(buffer_ref.len());
                chunk_data.extend_from_slice(&buffer_ref[0..bytes_to_copy]);
                bytes_written += bytes_to_copy;
            }
            Ok::<_, std::io::Error>(Bytes::from(chunk_data))
        }
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
        match chunk {
            Ok(data) => {
                total_bytes += data.len();
                // Immediately drop data to minimize memory pressure
                drop(data); // Explicit but just in case
            }
            Err(_) => break,
        }
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
