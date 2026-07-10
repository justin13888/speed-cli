//! UDP blaster: a fixed-rate, no-retransmit UDP test protocol with
//! receiver-side loss / OOO / jitter accounting. See `protocol` for the
//! wire format. The previous STP / BBR implementation has been retired
//! - see the Phase 3 entry in the project plan for context.

pub mod batch;
pub mod client;
pub mod protocol;
pub mod server;

use socket2::SockRef;
use tokio::net::UdpSocket;
use tracing::debug;

/// Requested UDP socket buffer size, per direction. The kernel clamps this to
/// `net.core.{rmem,wmem}_max` (and typically doubles the granted value for
/// bookkeeping), so we ask generously. The default socket buffer (~200 KB on
/// Linux) overflows and silently drops datagrams once a blaster run pushes past
/// a few hundred Mbps — the dominant reason raw UDP trails QUIC, which sizes its
/// own sockets up the same way.
pub const SOCKET_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Enlarge a UDP socket's send and receive buffers in place. Best-effort: a
/// platform that rejects or clamps the request keeps whatever it granted; we
/// never fail a test over it, but a clamp is surfaced once as an actionable
/// warning (sysctl hint) because it silently caps measurable throughput.
pub fn tune_socket_buffers(socket: &UdpSocket, requested: usize) {
    let sock = SockRef::from(socket);
    if let Err(e) = sock.set_recv_buffer_size(requested) {
        debug!("UDP set_recv_buffer_size({requested}) failed: {e}");
    }
    if let Err(e) = sock.set_send_buffer_size(requested) {
        debug!("UDP set_send_buffer_size({requested}) failed: {e}");
    }
    let rcv = sock.recv_buffer_size().unwrap_or(0);
    let snd = sock.send_buffer_size().unwrap_or(0);
    debug!("UDP socket buffers granted: recv={rcv} B, send={snd} B (requested {requested} B each)");
    crate::utils::net::warn_once_on_clamp("UDP socket buffers", requested, rcv, snd);
}
