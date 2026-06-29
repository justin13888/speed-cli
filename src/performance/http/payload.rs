//! Shared download payload buffer.
//!
//! A single 1 MB random buffer is generated once at process start and
//! reused by every server-side download path (axum HTTP, HTTP/3, raw
//! QUIC). Random content stops compressing middleboxes (some VPNs,
//! modems) from deflating the stream and inflating reported throughput.
//! 1 MB is small enough to generate in ~ms yet large enough that the
//! repeat path below rarely runs more than a few iterations.

use std::sync::LazyLock;

use bytes::Bytes;
use futures::{Stream, StreamExt as _};

/// Process-wide random download buffer. `Bytes` is itself a cheap-to-clone
/// reference-counted buffer, so no extra `Arc` wrapper is needed.
pub static RAND_BUFFER: LazyLock<Bytes> = LazyLock::new(|| {
    use rand::RngCore as _;
    let mut buf = vec![0u8; 1024 * 1024]; // 1 MB
    rand::rng().fill_bytes(&mut buf);
    Bytes::from(buf)
});

/// Produce one download chunk of exactly `len` bytes drawn from
/// [`RAND_BUFFER`], repeating the buffer when `len` exceeds its size.
///
/// The common case (`len <= 1 MB`) is zero-copy: `Bytes::slice` returns a view
/// into the shared buffer, just bumping its refcount. Only the rare oversized
/// chunk allocates and copies.
pub fn chunk_of(len: usize) -> Bytes {
    let buf = &*RAND_BUFFER;
    if len <= buf.len() {
        buf.slice(0..len)
    } else {
        let mut data = Vec::with_capacity(len);
        while data.len() < len {
            let n = (len - data.len()).min(buf.len());
            data.extend_from_slice(&buf[0..n]);
        }
        Bytes::from(data)
    }
}

/// A stream of chunks summing to `total_size` bytes, each at most
/// `chunk_size`. Shared by the axum and HTTP/3 download handlers.
pub fn download_stream(
    total_size: usize,
    chunk_size: usize,
) -> impl Stream<Item = std::result::Result<Bytes, std::io::Error>> {
    let chunk_size = chunk_size.max(1);
    let chunks = total_size.div_ceil(chunk_size);
    futures::stream::iter(0..chunks).map(move |i| {
        let bytes_sent = i * chunk_size;
        let remaining = total_size.saturating_sub(bytes_sent);
        let this_chunk = chunk_size.min(remaining);
        Ok(chunk_of(this_chunk))
    })
}
