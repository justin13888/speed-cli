//! Raw QUIC test server.
//!
//! Accepts QUIC connections, then per connection accepts bidirectional
//! streams. Each stream begins with a one-byte command and is handled
//! exactly like a raw-TCP connection.

use std::net::SocketAddr;
use std::sync::Arc;

use eyre::{Result, eyre};
use quinn::{RecvStream, SendStream};
use tokio_util::sync::CancellationToken;

use crate::performance::handshake::server_hello_io;
use crate::performance::http::payload::chunk_of;
use crate::performance::quic::QUIC_RAW_ALPN;
use crate::utils::tls::TlsMaterial;

/// Configuration for the raw-QUIC server.
pub struct QuicServerConfig {
    /// Shared TLS material (QUIC mandates TLS).
    pub tls: TlsMaterial,
    /// Per-write chunk size for download/full-duplex sends.
    pub buffer_size: usize,
}

/// Bind a QUIC endpoint for the raw-QUIC test on `addr` (port `0` =>
/// OS-assigned). Returns the endpoint and the real local port.
pub fn bind_quic(addr: SocketAddr, cfg: &QuicServerConfig) -> Result<(quinn::Endpoint, u16)> {
    let server_config = cfg.tls.server_config(&[QUIC_RAW_ALPN])?;
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(server_config)
        .map_err(|e| eyre!("raw-QUIC crypto config: {e}"))?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    let endpoint = quinn::Endpoint::server(server_config, addr)
        .map_err(|e| eyre!("raw-QUIC endpoint bind on {addr}: {e}"))?;
    let port = endpoint
        .local_addr()
        .map_err(|e| eyre!("raw-QUIC endpoint local_addr: {e}"))?
        .port();
    Ok((endpoint, port))
}

/// Run the raw-QUIC accept loop on a pre-bound endpoint until `cancel`
/// fires.
pub async fn run_quic_server(
    endpoint: quinn::Endpoint,
    cfg: QuicServerConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let cfg = Arc::new(cfg);
    tracing::info!("raw-QUIC server listening on {}", endpoint.local_addr()?);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("raw-QUIC server received shutdown signal, draining...");
                break;
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let cfg = cfg.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(incoming, cfg, cancel).await {
                        tracing::debug!("raw-QUIC connection ended: {e}");
                    }
                });
            }
        }
    }

    endpoint.close(0u32.into(), b"shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

async fn handle_connection(
    incoming: quinn::Incoming,
    cfg: Arc<QuicServerConfig>,
    cancel: CancellationToken,
) -> Result<()> {
    let conn = incoming.await.map_err(|e| eyre!("QUIC handshake: {e}"))?;
    let peer = conn.remote_address();
    tracing::debug!("raw-QUIC connection from {peer}");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = conn.accept_bi() => {
                match accepted {
                    Ok((send, recv)) => {
                        let cfg = cfg.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_stream(send, recv, peer, cfg).await {
                                tracing::debug!("raw-QUIC stream ended: {e}");
                            }
                        });
                    }
                    // Connection closed / drained — stop accepting.
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

async fn handle_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    peer: SocketAddr,
    cfg: Arc<QuicServerConfig>,
) -> Result<()> {
    let mut cmd = [0u8; 1];
    recv.read_exact(&mut cmd)
        .await
        .map_err(|e| eyre!("read command byte: {e}"))?;

    match cmd[0] {
        b'U' => handle_upload(&mut recv).await,
        b'D' => handle_download(&mut send, cfg.buffer_size).await,
        b'F' => handle_full_duplex(&mut send, &mut recv, cfg.buffer_size).await,
        b'P' => handle_ping(&mut send, &mut recv).await,
        b'H' => {
            server_hello_io(&mut recv, &mut send, peer).await?;
            let _ = send.finish();
            Ok(())
        }
        other => Err(eyre!("unknown raw-QUIC command byte: {other}")),
    }
}

/// Upload: drain the receive stream until the client finishes it.
async fn handle_upload(recv: &mut RecvStream) -> Result<()> {
    let mut buf = vec![0u8; 131_072];
    while let Some(_n) = recv
        .read(&mut buf)
        .await
        .map_err(|e| eyre!("upload read: {e}"))?
    {}
    Ok(())
}

/// Download: stream random data until the client stops the stream.
async fn handle_download(send: &mut SendStream, buffer_size: usize) -> Result<()> {
    let chunk = chunk_of(buffer_size.max(1));
    loop {
        if send.write_chunk(chunk.clone()).await.is_err() {
            break;
        }
    }
    Ok(())
}

/// Full-duplex: read and write concurrently until either side ends.
async fn handle_full_duplex(
    send: &mut SendStream,
    recv: &mut RecvStream,
    buffer_size: usize,
) -> Result<()> {
    let chunk = chunk_of(buffer_size.max(1));
    let write_fut = async {
        loop {
            if send.write_chunk(chunk.clone()).await.is_err() {
                break;
            }
        }
    };
    let read_fut = async {
        let mut buf = vec![0u8; 131_072];
        while let Ok(Some(_)) = recv.read(&mut buf).await {}
    };
    tokio::join!(write_fut, read_fut);
    Ok(())
}

/// Ping: echo 8-byte timestamps back to the client.
async fn handle_ping(send: &mut SendStream, recv: &mut RecvStream) -> Result<()> {
    let mut buf = [0u8; 8];
    while recv.read_exact(&mut buf).await.is_ok() {
        if send.write_all(&buf).await.is_err() {
            break;
        }
    }
    Ok(())
}
