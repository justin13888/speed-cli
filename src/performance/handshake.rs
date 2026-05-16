//! Transport-agnostic identity handshake (`'H'` command).
//!
//! The wire shape is the same regardless of transport (TCP stream or
//! QUIC bidirectional stream):
//! - Client writes `'H'` (1 byte) + `u32 BE` length prefix + CBOR
//!   `PeerIdentity`.
//! - Server replies with `u32 BE` + CBOR `PeerIdentity` then `u32 BE` +
//!   CBOR `SocketAddr` (the address it observed for the peer).
//!
//! The helpers are generic over split reader/writer halves so the same
//! code drives a `TcpStream` (via `split()`) and a QUIC bidi stream
//! (`SendStream` + `RecvStream`).

use std::net::SocketAddr;
use std::time::Duration;

use eyre::{Result, eyre};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::report::PeerIdentity;

/// Maximum time allowed for the whole handshake exchange.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);

/// Sanity cap on an embedded CBOR blob — identities are tiny (<200 B);
/// anything large is a malformed or hostile peer.
const MAX_BLOB_BYTES: u32 = 16 * 1024;

/// Outcome of the client side of the handshake.
pub struct ClientHelloResult {
    pub server_identity: PeerIdentity,
    pub observed_client_addr: SocketAddr,
}

/// Read a `u32 BE` length-prefixed blob.
pub async fn read_blob<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_BLOB_BYTES {
        return Err(eyre!("handshake blob too large: {len} bytes"));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a `u32 BE` length-prefixed blob.
pub async fn write_blob<W>(writer: &mut W, bytes: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let len = u32::try_from(bytes.len()).map_err(|_| eyre!("handshake blob too large"))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(bytes).await?;
    Ok(())
}

/// CBOR-encode a value.
pub fn encode_cbor<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(|e| eyre!("cbor encode: {e}"))?;
    Ok(buf)
}

/// CBOR-decode a value.
pub fn decode_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    ciborium::from_reader(bytes).map_err(|e| eyre!("cbor decode: {e}"))
}

/// Client side of the handshake over already-split stream halves. The
/// `'H'` command byte is written here.
pub async fn client_hello_io<R, W>(
    reader: &mut R,
    writer: &mut W,
    identity: &PeerIdentity,
) -> Result<ClientHelloResult>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    writer.write_all(b"H").await?;
    write_blob(writer, &encode_cbor(identity)?).await?;
    writer.flush().await?;

    let server_identity: PeerIdentity = decode_cbor(&read_blob(reader).await?)?;
    let observed_client_addr: SocketAddr = decode_cbor(&read_blob(reader).await?)?;

    Ok(ClientHelloResult {
        server_identity,
        observed_client_addr,
    })
}

/// Server side of the handshake over already-split stream halves. The
/// `'H'` command byte has already been consumed by the caller's command
/// dispatcher.
pub async fn server_hello_io<R, W>(reader: &mut R, writer: &mut W, peer: SocketAddr) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // The client identity is informational on the server today.
    let _client_identity: PeerIdentity = decode_cbor(&read_blob(reader).await?)?;

    write_blob(writer, &encode_cbor(&PeerIdentity::local())?).await?;
    write_blob(writer, &encode_cbor(&peer)?).await?;
    writer.flush().await?;
    Ok(())
}
