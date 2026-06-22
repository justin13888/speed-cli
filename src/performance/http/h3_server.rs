//! HTTP/3 (over QUIC) server.
//!
//! Exposes the same test surface as the axum HTTP server
//! (`/download`, `/upload`, `/latency`, `/info`, `/health`) but over
//! HTTP/3. Runs on its own QUIC endpoint with a distinct ephemeral UDP
//! port so it can never be confused with the raw-QUIC listener (the two
//! also use different ALPN identifiers).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{Method, Response, StatusCode, header};
use bytes::{Buf, Bytes};
use eyre::{Result, eyre};
use tokio_util::sync::CancellationToken;

use crate::constants::DEFAULT_CHUNK_SIZE;
use crate::performance::http::payload;
use crate::performance::http::server::{SERVER_ID_HEADER, server_identity_header_value};
use crate::utils::tls::TlsMaterial;

/// Configuration for the HTTP/3 server.
pub struct Http3ServerConfig {
    /// Maximum accepted upload size, in bytes.
    pub max_upload_size: usize,
    /// Shared TLS material. HTTP/3 mandates TLS.
    pub tls: TlsMaterial,
}

/// Bind a QUIC endpoint for HTTP/3 on `addr` (port `0` => OS-assigned).
/// Returns the endpoint and the real local port for the manifest.
pub fn bind_h3(addr: SocketAddr, cfg: &Http3ServerConfig) -> Result<(quinn::Endpoint, u16)> {
    let server_config = cfg.tls.server_config(&[b"h3"])?;
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(server_config)
        .map_err(|e| eyre!("HTTP/3 QUIC crypto config: {e}"))?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    let endpoint = quinn::Endpoint::server(server_config, addr)
        .map_err(|e| eyre!("HTTP/3 endpoint bind on {addr}: {e}"))?;
    let port = endpoint
        .local_addr()
        .map_err(|e| eyre!("HTTP/3 endpoint local_addr: {e}"))?
        .port();
    Ok((endpoint, port))
}

/// Run the HTTP/3 accept loop on a pre-bound endpoint until `cancel`
/// fires.
pub async fn run_h3_server(
    endpoint: quinn::Endpoint,
    cfg: Http3ServerConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let cfg = Arc::new(cfg);
    tracing::info!("HTTP/3 server listening on {}", endpoint.local_addr()?);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("HTTP/3 server received shutdown signal, draining...");
                break;
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let cfg = cfg.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(incoming, cfg).await {
                        tracing::debug!("HTTP/3 connection ended: {e}");
                    }
                });
            }
        }
    }

    endpoint.close(0u32.into(), b"shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

async fn handle_connection(incoming: quinn::Incoming, cfg: Arc<Http3ServerConfig>) -> Result<()> {
    let conn = incoming.await.map_err(|e| eyre!("QUIC handshake: {e}"))?;
    let mut h3_conn = h3::server::Connection::<_, Bytes>::new(h3_quinn::Connection::new(conn))
        .await
        .map_err(|e| eyre!("h3 connection setup: {e}"))?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let cfg = cfg.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_request(resolver, cfg).await {
                        tracing::debug!("HTTP/3 request error: {e}");
                    }
                });
            }
            Ok(None) => break,
            Err(e) => {
                tracing::debug!("h3 accept error: {e}");
                break;
            }
        }
    }
    Ok(())
}

async fn handle_request(
    resolver: h3::server::RequestResolver<h3_quinn::Connection, Bytes>,
    cfg: Arc<Http3ServerConfig>,
) -> Result<()> {
    let (req, mut stream) = resolver
        .resolve_request()
        .await
        .map_err(|e| eyre!("resolve request: {e}"))?;

    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    match (&method, path.as_str()) {
        (&Method::GET, "/download") => {
            let (size, chunk_size) = parse_download_query(query.as_deref());
            let resp = base_response()
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, size.to_string())
                .body(())
                .map_err(|e| eyre!("build response: {e}"))?;
            stream
                .send_response(resp)
                .await
                .map_err(|e| eyre!("send response: {e}"))?;
            let mut sent = 0usize;
            while sent < size {
                let n = chunk_size.min(size - sent);
                stream
                    .send_data(payload::chunk_of(n))
                    .await
                    .map_err(|e| eyre!("send data: {e}"))?;
                sent += n;
            }
        }
        (&Method::POST, "/upload") => {
            let mut total: u64 = 0;
            while let Some(buf) = stream
                .recv_data()
                .await
                .map_err(|e| eyre!("recv data: {e}"))?
            {
                total += buf.remaining() as u64;
                if total > cfg.max_upload_size as u64 {
                    break;
                }
            }
            let resp = base_response()
                .body(())
                .map_err(|e| eyre!("build response: {e}"))?;
            stream
                .send_response(resp)
                .await
                .map_err(|e| eyre!("send response: {e}"))?;
            stream
                .send_data(Bytes::from(total.to_string()))
                .await
                .map_err(|e| eyre!("send data: {e}"))?;
        }
        (&Method::GET | &Method::HEAD, "/latency") => {
            let resp = base_response()
                .body(())
                .map_err(|e| eyre!("build response: {e}"))?;
            stream
                .send_response(resp)
                .await
                .map_err(|e| eyre!("send response: {e}"))?;
        }
        (&Method::GET, "/info") => {
            send_text(&mut stream, StatusCode::OK, "speed-cli HTTP/3 server\n").await?;
        }
        (&Method::GET, "/health") => {
            send_text(&mut stream, StatusCode::OK, "ok").await?;
        }
        _ => {
            send_text(&mut stream, StatusCode::NOT_FOUND, "not found").await?;
        }
    }

    stream
        .finish()
        .await
        .map_err(|e| eyre!("finish stream: {e}"))?;
    Ok(())
}

/// Response builder seeded with the server-identity header so HTTP/3
/// clients can read the server's [`PeerIdentity`](crate::report::PeerIdentity)
/// just like the axum path.
fn base_response() -> axum::http::response::Builder {
    Response::builder()
        .status(StatusCode::OK)
        .header(SERVER_ID_HEADER, server_identity_header_value())
}

async fn send_text(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    status: StatusCode,
    body: &str,
) -> Result<()> {
    let resp = base_response()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(())
        .map_err(|e| eyre!("build response: {e}"))?;
    stream
        .send_response(resp)
        .await
        .map_err(|e| eyre!("send response: {e}"))?;
    stream
        .send_data(Bytes::copy_from_slice(body.as_bytes()))
        .await
        .map_err(|e| eyre!("send data: {e}"))?;
    Ok(())
}

/// Parse `size` and `chunk_size` from a `/download` query string.
fn parse_download_query(query: Option<&str>) -> (usize, usize) {
    let mut size = 0usize;
    let mut chunk_size = DEFAULT_CHUNK_SIZE;
    if let Some(q) = query {
        for pair in q.split('&') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            match k {
                "size" => size = v.parse().unwrap_or(0),
                "chunk_size" => chunk_size = v.parse().unwrap_or(DEFAULT_CHUNK_SIZE).max(1),
                _ => {}
            }
        }
    }
    (size, chunk_size)
}
