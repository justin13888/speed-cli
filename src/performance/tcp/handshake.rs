//! TCP `'H'` command: a tiny in-band identity handshake.
//!
//! The wire shape is shared with every other transport — see
//! [`crate::performance::handshake`]. This module is just the
//! TCP-specific glue: connecting a fresh `TcpStream`, splitting it, and
//! running the transport-agnostic exchange.
//!
//! Failure modes (server doesn't speak `'H'`, mismatched binary) are
//! non-fatal: callers swallow the error and proceed without a populated
//! `RemoteView`.

use std::net::SocketAddr;

use eyre::Result;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::performance::handshake::{HELLO_TIMEOUT, client_hello_io, server_hello_io};
use crate::report::PeerIdentity;

pub use crate::performance::handshake::ClientHelloResult;

/// Run the client side of the `'H'` exchange against a running server.
/// Returns `None` on any failure (timeout, malformed, server doesn't
/// speak `'H'`); the caller should record `RemoteView::default()` and
/// move on.
pub async fn client_hello(addr: &str, identity: &PeerIdentity) -> Option<ClientHelloResult> {
    let stream = TcpStream::connect(addr).await.ok()?;
    match timeout(HELLO_TIMEOUT, client_hello_inner(stream, identity)).await {
        Ok(Ok(r)) => Some(r),
        _ => None,
    }
}

async fn client_hello_inner(
    mut stream: TcpStream,
    identity: &PeerIdentity,
) -> Result<ClientHelloResult> {
    let (mut read_half, mut write_half) = stream.split();
    let result = client_hello_io(&mut read_half, &mut write_half, identity).await?;
    let _ = stream.shutdown().await;
    Ok(result)
}

/// Server-side: handle a single `'H'` exchange. The `'H'` byte has
/// already been consumed by the caller.
pub async fn server_hello(stream: &mut TcpStream, peer: SocketAddr) -> Result<()> {
    let (mut read_half, mut write_half) = stream.split();
    server_hello_io(&mut read_half, &mut write_half, peer).await?;
    let _ = stream.shutdown().await;
    Ok(())
}
