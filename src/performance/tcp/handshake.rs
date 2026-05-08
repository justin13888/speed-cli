//! TCP `'H'` command: a tiny in-band identity handshake.
//!
//! Wire shape:
//! - Client connects, writes `'H'` (1 byte) + `u32 BE` length prefix +
//!   CBOR-encoded `PeerIdentity` for the client.
//! - Server reads the prefix and identity, replies with `u32 BE` +
//!   CBOR `PeerIdentity` for the server, followed by `u32 BE` + CBOR
//!   `SocketAddr` (the address the server observed for the peer).
//! - Both sides close the connection. The actual throughput test
//!   re-opens a fresh `'D'`/`'U'`/`'F'` connection.
//!
//! Failure modes (e.g. server doesn't speak `'H'`, mismatched binary)
//! are non-fatal: callers swallow the error and proceed without a
//! populated `RemoteView`.

use std::net::SocketAddr;
use std::time::Duration;

use eyre::{Result, eyre};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::report::PeerIdentity;

const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// Sanity cap on the size of an embedded CBOR blob — small identities
/// are tiny (<200 B); anything large is a malformed peer.
const MAX_BLOB_BYTES: u32 = 16 * 1024;

async fn read_blob<R>(stream: &mut R) -> Result<Vec<u8>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_BLOB_BYTES {
        return Err(eyre!("hello blob too large: {len} bytes"));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_blob<W>(stream: &mut W, bytes: &[u8]) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let len = u32::try_from(bytes.len()).map_err(|_| eyre!("hello blob too large"))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(bytes).await?;
    Ok(())
}

fn encode_cbor<T: serde::Serialize>(v: &T) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::into_writer(v, &mut buf).map_err(|e| eyre!("cbor encode: {e}"))?;
    Ok(buf)
}

fn decode_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    ciborium::from_reader(bytes).map_err(|e| eyre!("cbor decode: {e}"))
}

/// Outcome of the client-side handshake. Either side of the wire may
/// fail independently — the caller decides what to do with the result.
pub struct ClientHelloResult {
    pub server_identity: PeerIdentity,
    pub observed_client_addr: SocketAddr,
}

/// Run the client side of the `'H'` exchange against a running server.
/// Returns `None` on any failure (timeout, malformed, server doesn't
/// speak `'H'`); the caller should record `RemoteView::default()` and
/// move on.
pub async fn client_hello(addr: &str, identity: &PeerIdentity) -> Option<ClientHelloResult> {
    let stream = match TcpStream::connect(addr).await {
        Ok(s) => s,
        Err(_) => return None,
    };
    match timeout(HELLO_TIMEOUT, client_hello_inner(stream, identity)).await {
        Ok(Ok(r)) => Some(r),
        _ => None,
    }
}

async fn client_hello_inner(
    mut stream: TcpStream,
    identity: &PeerIdentity,
) -> Result<ClientHelloResult> {
    stream.write_all(b"H").await?;
    let id_bytes = encode_cbor(identity)?;
    write_blob(&mut stream, &id_bytes).await?;

    let server_id_bytes = read_blob(&mut stream).await?;
    let server_identity: PeerIdentity = decode_cbor(&server_id_bytes)?;

    let addr_bytes = read_blob(&mut stream).await?;
    let observed_client_addr: SocketAddr = decode_cbor(&addr_bytes)?;

    let _ = stream.shutdown().await;
    Ok(ClientHelloResult {
        server_identity,
        observed_client_addr,
    })
}

/// Server-side: handle a single `'H'` exchange. The `'H'` byte has
/// already been consumed by the caller. Reads the client's identity,
/// writes the server's identity and the observed peer address.
pub async fn server_hello(stream: &mut TcpStream, peer: SocketAddr) -> Result<()> {
    // Reading the client identity is purely informational on the
    // server side today — the server doesn't keep it. (It could, in
    // a future bidirectional report.)
    let client_id_bytes = read_blob(stream).await?;
    let _client_identity: PeerIdentity = decode_cbor(&client_id_bytes)?;

    let server_identity = PeerIdentity::local();
    let server_id_bytes = encode_cbor(&server_identity)?;
    write_blob(stream, &server_id_bytes).await?;

    let addr_bytes = encode_cbor(&peer)?;
    write_blob(stream, &addr_bytes).await?;

    let _ = stream.shutdown().await;
    Ok(())
}
