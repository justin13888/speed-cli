use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Query},
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use eyre::{Context as _, Result};
use futures::StreamExt as _;
use hyper::server::conn::{http1, http2};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use serde::Deserialize;
use std::sync::Once;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::report::PeerIdentity;

pub const SERVER_ID_HEADER: &str = "x-speed-cli-server-id";

pub fn server_identity_header_value() -> HeaderValue {
    // Encode the local PeerIdentity as base64-CBOR once at startup.
    // Header values are ASCII-safe; base64 (URL-safe, no padding) keeps
    // the wire compact and avoids escaping concerns. Falls back to an
    // empty value on the (impossible-in-practice) failure path so the
    // header layer can still be installed.
    let mut buf = Vec::new();
    if ciborium::into_writer(&PeerIdentity::local(), &mut buf).is_ok() {
        let encoded = base64_urlsafe(&buf);
        if let Ok(v) = HeaderValue::from_str(&encoded) {
            return v;
        }
    }
    HeaderValue::from_static("")
}

fn base64_urlsafe(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let chunks = input.chunks_exact(3);
    let rem = chunks.remainder();
    for c in chunks {
        let n = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
    }
    match rem {
        [a] => {
            let n = (*a as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        }
        [a, b] => {
            let n = ((*a as u32) << 16) | ((*b as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }
    out
}

pub fn decode_base64_urlsafe(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let chunks = bytes.chunks(4);
    for c in chunks {
        if c.len() < 2 {
            return None;
        }
        let a = val(c[0])?;
        let b = val(c[1])?;
        let cc = if c.len() > 2 { val(c[2])? } else { 0 };
        let d = if c.len() > 3 { val(c[3])? } else { 0 };
        let n = ((a as u32) << 18) | ((b as u32) << 12) | ((cc as u32) << 6) | d as u32;
        out.push(((n >> 16) & 0xff) as u8);
        if c.len() > 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if c.len() > 3 {
            out.push((n & 0xff) as u8);
        }
    }
    Some(out)
}

use crate::constants::{
    DEFAULT_CHUNK_SIZE, HTTP2_CONNECTION_WINDOW, HTTP2_MAX_FRAME_SIZE, HTTP2_MAX_SEND_BUF,
    HTTP2_STREAM_WINDOW,
};

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    });
}

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    /// Enable cors. Usually should be true.
    pub enable_cors: bool,
    /// Max upload size in bytes
    pub max_upload_size: usize,
}

/// Which cleartext HTTP protocol a listener speaks. Each protocol gets
/// its own listener and is served *strictly* — a client that speaks the
/// wrong protocol fails its connection handshake loudly instead of
/// being silently negotiated onto the other protocol (which is what
/// `axum::serve`'s auto-detection would do, hiding measurement bugs).
#[derive(Debug, Clone, Copy)]
enum CleartextProto {
    /// HTTP/1.1 only.
    Http1,
    /// HTTP/2 cleartext (h2c), prior-knowledge only.
    H2c,
}

/// Serve the test router over a single cleartext HTTP protocol on a
/// pre-bound listener, gracefully draining when `cancel` fires.
async fn run_cleartext(
    listener: TcpListener,
    config: HttpServerConfig,
    cancel: CancellationToken,
    proto: CleartextProto,
) -> Result<()> {
    let router = create_router(config.enable_cors, config.max_upload_size);
    let graceful = GracefulShutdown::new();

    tracing::info!("{:?} server listening on {}", proto, listener.local_addr()?);

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, _peer) = match accept {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::error!("{proto:?} accept error: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                let io = TokioIo::new(stream);
                let svc = TowerToHyperService::new(router.clone());
                match proto {
                    CleartextProto::Http1 => {
                        let conn = http1::Builder::new().serve_connection(io, svc);
                        let watched = graceful.watch(conn);
                        tokio::spawn(async move {
                            if let Err(e) = watched.await {
                                tracing::debug!("HTTP/1.1 connection error: {e}");
                            }
                        });
                    }
                    CleartextProto::H2c => {
                        // Match the client's enlarged flow-control windows so h2c
                        // throughput isn't pinned at the 64 KiB h2 defaults.
                        let mut builder = http2::Builder::new(TokioExecutor::new());
                        builder
                            .initial_stream_window_size(HTTP2_STREAM_WINDOW)
                            .initial_connection_window_size(HTTP2_CONNECTION_WINDOW)
                            .max_frame_size(HTTP2_MAX_FRAME_SIZE)
                            // Raise the per-connection send buffer off hyper's
                            // 400 KB default; all h2c streams share it.
                            .max_send_buf_size(HTTP2_MAX_SEND_BUF);
                        let conn = builder.serve_connection(io, svc);
                        let watched = graceful.watch(conn);
                        tokio::spawn(async move {
                            if let Err(e) = watched.await {
                                tracing::debug!("h2c connection error: {e}");
                            }
                        });
                    }
                }
            }
            _ = cancel.cancelled() => {
                tracing::info!("{proto:?} server received shutdown signal, draining...");
                break;
            }
        }
    }

    tokio::select! {
        _ = graceful.shutdown() => {}
        _ = tokio::time::sleep(Duration::from_secs(10)) => {
            tracing::warn!("{proto:?} server: graceful drain timed out");
        }
    }
    Ok(())
}

/// Runs the HTTP/1.1-only server on a pre-bound listener. An h2c
/// prior-knowledge client's preface is not valid HTTP/1.1, so it fails
/// loudly here rather than being silently served h2c.
pub async fn run_http1_server(
    listener: TcpListener,
    config: HttpServerConfig,
    cancel: CancellationToken,
) -> Result<()> {
    run_cleartext(listener, config, cancel, CleartextProto::Http1).await
}

/// Runs the h2c-only (HTTP/2 cleartext, prior-knowledge) server on a
/// pre-bound listener. An HTTP/1.1 client does not send the HTTP/2
/// connection preface, so it fails loudly here rather than being
/// silently served HTTP/1.1.
pub async fn run_h2c_server(
    listener: TcpListener,
    config: HttpServerConfig,
    cancel: CancellationToken,
) -> Result<()> {
    run_cleartext(listener, config, cancel, CleartextProto::H2c).await
}

/// Runs the HTTPS (HTTP/2 over TLS) server on a pre-bound listener,
/// gracefully shutting down when `cancel` fires. ALPN restricts the
/// listener to HTTP/2, so it is already strict.
pub async fn run_https_server(
    listener: std::net::TcpListener,
    tls_config: RustlsConfig,
    enable_cors: bool,
    max_upload_size: usize,
    cancel: CancellationToken,
) -> Result<()> {
    // Ensure crypto provider is initialized before using TLS
    ensure_crypto_provider();

    let app = create_router(enable_cors, max_upload_size);

    listener
        .set_nonblocking(true)
        .wrap_err("Failed to set HTTPS listener non-blocking")?;
    tracing::info!("HTTPS server listening on {}", listener.local_addr()?);

    let handle = axum_server::Handle::new();
    let handle_for_shutdown = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        cancel.cancelled().await;
        tracing::info!("HTTPS server received shutdown signal, draining...");
        handle_for_shutdown.graceful_shutdown(Some(Duration::from_secs(30)));
    });

    // Enlarge HTTP/2 flow-control windows (axum_server exposes the underlying
    // hyper-util auto builder) so HTTPS throughput isn't pinned at the 64 KiB
    // h2 defaults, matching the h2c and client configuration.
    let mut server = axum_server::from_tcp_rustls(listener, tls_config);
    server
        .http_builder()
        .http2()
        .initial_stream_window_size(HTTP2_STREAM_WINDOW)
        .initial_connection_window_size(HTTP2_CONNECTION_WINDOW)
        .max_frame_size(HTTP2_MAX_FRAME_SIZE)
        // Raise the per-connection send buffer off hyper's 400 KB default;
        // all multiplexed streams share it. Matches the h2c path.
        .max_send_buf_size(HTTP2_MAX_SEND_BUF);
    let result = server.handle(handle).serve(app.into_make_service()).await;

    shutdown_task.abort();
    result?;

    Ok(())
}

fn create_router(enable_cors: bool, max_upload_size: usize) -> Router {
    let mut router = Router::new()
        .route("/download", get(download_handler))
        .route("/upload", post(upload_handler))
        .route("/latency", get(latency_handler).head(latency_handler))
        .route("/info", get(info_handler))
        .route("/health", get(health_handler))
        .layer(DefaultBodyLimit::max(max_upload_size))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static(SERVER_ID_HEADER),
            server_identity_header_value(),
        ));

    if enable_cors {
        router = router.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::HEAD])
                .allow_headers(Any),
        );
    }

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
    let body = Body::from_stream(crate::performance::http::payload::download_stream(
        query.size,
        query.chunk_size,
    ));

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, query.size.to_string())
        .body(body)
    {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to build download response: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to build response",
            )
                .into_response()
        }
    }
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
    (StatusCode::OK, format!("{total_bytes}"))
}

async fn latency_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn info_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        "speed-cli HTTP server\nendpoints: /download /upload /latency /info /health\n",
    )
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
