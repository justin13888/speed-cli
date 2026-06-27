//! GSO/GRO batched UDP I/O via `quinn-udp`, with a transparent
//! per-datagram fallback where segmentation offload is unavailable.
//!
//! Generic Segmentation Offload (send) and Generic Receive Offload (recv)
//! are how `quinn` moves many MTU-sized datagrams per syscall — the main
//! reason raw QUIC out-throughputs a naive per-packet UDP loop. We borrow
//! the same machinery for the blaster's MTU-bound datagrams.
//!
//! Scope note: `quinn-udp` configures the socket for path-MTU discovery,
//! which sets the IPv4 *don't-fragment* bit. That is correct for MTU-sized
//! datagrams (QUIC never fragments) but would make the blaster's larger
//! (> MTU) datagram sizes fail to send. Callers therefore only route
//! MTU-sized sends through [`BatchIo::send_segmented`] and keep the plain
//! `send_to` path for larger datagrams. Receiving is unaffected — the
//! kernel reassembles fragments regardless — so the receive side can always
//! use [`BatchIo::recv_coalesced`].

use std::io::{self, IoSliceMut};
use std::net::SocketAddr;

use quinn_udp::{RecvMeta, Transmit, UdpSockRef, UdpSocketState};
use tokio::io::Interest;
use tokio::net::UdpSocket;

/// Largest single-datagram wire size we will hand to GSO: a 1500-byte
/// Ethernet MTU minus the 28-byte IPv4 + UDP headers. Datagrams at or below
/// this never need fragmentation, so the don't-fragment bit `quinn-udp` sets
/// is harmless. Anything larger stays on the plain, fragmentation-capable
/// path.
pub const MAX_OFFLOAD_DATAGRAM: usize = 1500 - 28;

/// Hard cap on segments coalesced into one GSO send, independent of what the
/// kernel advertises. Keeps the staging buffer bounded.
const MAX_BATCH_SEGMENTS: usize = 64;

/// Batched UDP I/O bound to one socket. Construct once per socket and reuse;
/// `UdpSocketState::new` performs the socket setup (enabling GRO, PMTU
/// probing, etc.) up front.
pub struct BatchIo {
    state: UdpSocketState,
    max_gso: usize,
}

impl BatchIo {
    /// Wrap `socket` for batched I/O. Returns an error only if the one-time
    /// socket configuration fails.
    pub fn new(socket: &UdpSocket) -> io::Result<Self> {
        let state = UdpSocketState::new(UdpSockRef::from(socket))?;
        let max_gso = state.max_gso_segments().max(1);
        Ok(Self { state, max_gso })
    }

    /// How many `seg_len`-byte datagrams to coalesce per GSO send: bounded by
    /// the kernel's GSO limit, the 64-segment cap, and the 64 KiB ceiling on
    /// a single GSO buffer. Returns 1 when offload is unavailable.
    pub fn segments_for(&self, seg_len: usize) -> usize {
        let seg_len = seg_len.max(1);
        let by_size = (u16::MAX as usize) / seg_len; // GSO buffer must fit in 64 KiB
        self.max_gso.min(MAX_BATCH_SEGMENTS).min(by_size).max(1)
    }

    /// Send `contents` to `dest` in a single syscall. When `contents` holds
    /// more than one `segment_size`-byte datagram the kernel segments it via
    /// GSO; a lone datagram is sent plainly. Awaits writability and retries
    /// on `WouldBlock`.
    pub async fn send_segmented(
        &self,
        socket: &UdpSocket,
        dest: SocketAddr,
        contents: &[u8],
        segment_size: usize,
    ) -> io::Result<()> {
        // Only advertise a segment size when there is more than one segment;
        // otherwise platforms without GSO would choke on the UDP_SEGMENT cmsg.
        let segment_size = (contents.len() > segment_size).then_some(segment_size);
        let transmit = Transmit {
            destination: dest,
            ecn: None,
            contents,
            segment_size,
            src_ip: None,
        };
        loop {
            socket.writable().await?;
            match socket.try_io(Interest::WRITABLE, || {
                self.state.send(UdpSockRef::from(socket), &transmit)
            }) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Receive one buffer's worth of datagrams. GRO may pack several into
    /// `buf`; the returned `stride` is the per-datagram size (the last one may
    /// be shorter). Slice `buf[..len]` into `stride`-sized chunks to recover
    /// individual datagrams. Awaits readability and retries on `WouldBlock`.
    ///
    /// Returns `(len, stride, source)`.
    pub async fn recv_coalesced(
        &self,
        socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<(usize, usize, SocketAddr)> {
        let mut meta = [RecvMeta::default()];
        loop {
            socket.readable().await?;
            let mut bufs = [IoSliceMut::new(buf)];
            match socket.try_io(Interest::READABLE, || {
                self.state.recv(UdpSockRef::from(socket), &mut bufs, &mut meta)
            }) {
                Ok(0) => continue,
                Ok(_) => {
                    let m = meta[0];
                    let stride = if m.stride == 0 { m.len } else { m.stride };
                    return Ok((m.len, stride.max(1), m.addr));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

/// Iterate the individual datagrams in a coalesced receive buffer.
pub fn split_datagrams(buf: &[u8], stride: usize) -> impl Iterator<Item = &[u8]> {
    buf.chunks(stride.max(1))
}
