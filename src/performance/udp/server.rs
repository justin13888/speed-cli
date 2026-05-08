//! UDP blaster server.
//!
//! Listens on a single UDP socket, demultiplexes by source address, and
//! handles three session kinds: download (server sends), upload (server
//! receives), and latency (PING/PONG echo). Sessions are bounded by
//! `MAX_SESSIONS` with LRU eviction and reaped when idle for longer
//! than `SESSION_IDLE_TIMEOUT`. There's no congestion control; pacing
//! during downloads is approximate (`tokio::time::sleep` granularity)
//! and is documented as such.

use bytes::Bytes;
use colored::*;
use eyre::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{ToSocketAddrs, UdpSocket};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};

use super::protocol::{BlasterPacket, Mode, ReceiveStats, now_us};
use crate::report::PeerIdentity;

/// Hard ceiling on per-source sessions. Beyond this we evict the
/// least-recently-active session.
pub const MAX_SESSIONS: usize = 10_000;
/// Sessions idle longer than this are reaped.
pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Cadence of the idle-eviction task.
const EVICTION_INTERVAL: Duration = Duration::from_secs(30);

/// Per-source session state. Discriminated by `mode` so we know how to
/// react to subsequent packets after the START handshake.
#[derive(Debug)]
struct Session {
    mode: Mode,
    last_seen: Instant,
    /// Receiver-side stats - meaningful for Upload sessions; we
    /// populate it incidentally on Download sessions too in case the
    /// client sends ACKs or similar in a future protocol extension.
    rx: ReceiveStats,
    /// For download sessions only: the configuration handed to us via
    /// START. The send loop runs in a background task keyed off the
    /// session.
    download_handle: Option<tokio::task::JoinHandle<()>>,
}

pub struct BlasterServer {
    socket: Arc<UdpSocket>,
    sessions: Arc<Mutex<HashMap<SocketAddr, Session>>>,
}

impl BlasterServer {
    pub async fn new(addr: impl ToSocketAddrs) -> Result<Self> {
        let socket = UdpSocket::bind(&addr).await?;
        Ok(Self {
            socket: Arc::new(socket),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn run(&self, cancel: CancellationToken) -> Result<()> {
        info!(
            "UDP blaster server listening on {}",
            self.socket.local_addr()?.to_string().green()
        );

        // Idle-eviction task
        let sessions = self.sessions.clone();
        let cancel_evict = cancel.clone();
        let evict_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(EVICTION_INTERVAL);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = Instant::now();
                        let mut s = sessions.lock();
                        let before = s.len();
                        s.retain(|_, sess| now.duration_since(sess.last_seen) < SESSION_IDLE_TIMEOUT);
                        let after = s.len();
                        if before != after {
                            debug!("Evicted {} idle UDP sessions ({} remaining)", before - after, after);
                        }
                    }
                    _ = cancel_evict.cancelled() => break,
                }
            }
        });

        let mut buf = vec![0u8; 4096];
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("UDP blaster server received shutdown signal");
                    break;
                }
                recv = self.socket.recv_from(&mut buf) => {
                    match recv {
                        Ok((n, peer)) => {
                            let data = &buf[..n];
                            self.handle_packet(peer, data).await;
                        }
                        Err(e) => {
                            error!("UDP recv error: {}", e);
                        }
                    }
                }
            }
        }

        evict_task.abort();

        // Drain any in-flight download tasks so we don't leave them
        // hammering closed sockets.
        let mut sessions = self.sessions.lock();
        for (_, sess) in sessions.drain() {
            if let Some(h) = sess.download_handle {
                h.abort();
            }
        }

        Ok(())
    }

    async fn handle_packet(&self, peer: SocketAddr, data: &[u8]) {
        let Some((packet, payload_len)) = BlasterPacket::decode(data) else {
            trace!("dropped non-blaster packet from {}", peer);
            return;
        };
        let recv_ts = now_us();

        match packet {
            BlasterPacket::Start {
                mode,
                target_rate_bps,
                payload_size,
                duration_ms,
            } => {
                self.handle_start(peer, mode, target_rate_bps, payload_size, duration_ms)
                    .await;
            }
            BlasterPacket::Data { seq, send_ts_us } => {
                self.handle_data(peer, seq, send_ts_us, payload_len as u64, recv_ts);
            }
            BlasterPacket::Fin => {
                self.handle_fin(peer).await;
            }
            BlasterPacket::Ping { send_ts_us } => {
                let pong = BlasterPacket::Pong { send_ts_us }.encode_to_vec(None);
                if let Err(e) = self.socket.send_to(&pong, peer).await {
                    debug!("pong send failed to {}: {}", peer, e);
                }
            }
            BlasterPacket::Hello { .. } => {
                self.handle_hello(peer).await;
            }
            BlasterPacket::Pong { .. }
            | BlasterPacket::Report { .. }
            | BlasterPacket::HelloAck { .. } => {
                // Server-bound; clients send these. Ignore.
            }
        }
    }

    async fn handle_hello(&self, peer: SocketAddr) {
        let mut id_buf = Vec::new();
        if ciborium::into_writer(&PeerIdentity::local(), &mut id_buf).is_err() {
            return;
        }
        let mut addr_buf = Vec::new();
        if ciborium::into_writer(&peer, &mut addr_buf).is_err() {
            return;
        }
        let ack = BlasterPacket::HelloAck {
            identity_cbor: id_buf,
            observed_client_addr_cbor: addr_buf,
            server_epoch_us: now_us(),
        };
        let bytes = ack.encode_to_vec(None);
        if let Err(e) = self.socket.send_to(&bytes, peer).await {
            debug!("HelloAck send failed to {}: {}", peer, e);
        }
    }

    async fn handle_start(
        &self,
        peer: SocketAddr,
        mode: Mode,
        target_rate_bps: u64,
        payload_size: u32,
        duration_ms: u64,
    ) {
        // Admit-or-evict. We also reset any prior session for this peer
        // so a re-tested client gets fresh stats.
        {
            let mut sessions = self.sessions.lock();
            if let Some(prev) = sessions.remove(&peer)
                && let Some(h) = prev.download_handle
            {
                h.abort();
            }
            if !sessions.contains_key(&peer)
                && sessions.len() >= MAX_SESSIONS
                && let Some(victim) = sessions
                    .iter()
                    .min_by_key(|(_, s)| s.last_seen)
                    .map(|(a, _)| *a)
            {
                debug!("session cap reached, evicting LRU {} for {}", victim, peer);
                if let Some(s) = sessions.remove(&victim)
                    && let Some(h) = s.download_handle
                {
                    h.abort();
                }
            }
            sessions.insert(
                peer,
                Session {
                    mode,
                    last_seen: Instant::now(),
                    rx: ReceiveStats::default(),
                    download_handle: None,
                },
            );
        }

        info!(
            "blaster START from {} mode={:?} rate={} bps payload={} duration={}ms",
            peer.to_string().cyan(),
            mode,
            target_rate_bps,
            payload_size,
            duration_ms
        );

        if mode == Mode::Download {
            // Spawn a sender task that runs for the requested duration.
            let socket = self.socket.clone();
            let sessions = self.sessions.clone();
            let handle = tokio::spawn(download_sender(
                socket,
                peer,
                target_rate_bps,
                payload_size as usize,
                Duration::from_millis(duration_ms),
                sessions.clone(),
            ));
            if let Some(s) = self.sessions.lock().get_mut(&peer) {
                s.download_handle = Some(handle);
            }
        }
    }

    fn handle_data(
        &self,
        peer: SocketAddr,
        seq: u64,
        send_ts_us: u64,
        payload_bytes: u64,
        recv_ts_us: u64,
    ) {
        let mut sessions = self.sessions.lock();
        let Some(sess) = sessions.get_mut(&peer) else {
            // No START seen for this peer; ignore. We deliberately do
            // *not* auto-create a session on bare DATA, that was the
            // STP-era DoS vector we fixed earlier.
            return;
        };
        sess.last_seen = Instant::now();
        sess.rx.record(seq, payload_bytes, send_ts_us, recv_ts_us);
    }

    async fn handle_fin(&self, peer: SocketAddr) {
        let report = {
            let mut sessions = self.sessions.lock();
            let Some(sess) = sessions.remove(&peer) else {
                return;
            };
            if let Some(h) = sess.download_handle {
                h.abort();
            }
            BlasterPacket::Report {
                received: sess.rx.received,
                bytes_received: sess.rx.bytes_received,
                lost: sess.rx.lost(),
                out_of_order: sess.rx.out_of_order,
                jitter_us: sess.rx.jitter_us(),
                duplicates: sess.rx.duplicates,
            }
        };
        let bytes = report.encode_to_vec(None);
        // Send a few copies to mitigate report-packet loss on lossy
        // links. The client deduplicates by ignoring repeated REPORTs.
        for _ in 0..3 {
            let _ = self.socket.send_to(&bytes, peer).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Run the server-side sender for a Download session.
async fn download_sender(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    target_rate_bps: u64,
    payload_size: usize,
    duration: Duration,
    sessions: Arc<Mutex<HashMap<SocketAddr, Session>>>,
) {
    use rand::RngCore as _;
    let mut payload = vec![0u8; payload_size];
    rand::rng().fill_bytes(&mut payload);

    let inter_packet_delay = if target_rate_bps > 0 {
        // bytes per second from bps; seconds per packet from that.
        let bytes_per_sec = target_rate_bps as f64 / 8.0;
        let secs_per_packet = (payload_size as f64) / bytes_per_sec.max(1.0);
        Some(Duration::from_secs_f64(secs_per_packet))
    } else {
        None // saturate
    };

    let start = Instant::now();
    let mut seq: u64 = 1;
    while start.elapsed() < duration {
        // Bail if the session was evicted (e.g., FIN received).
        if !sessions.lock().contains_key(&peer) {
            break;
        }

        let pkt = BlasterPacket::Data {
            seq,
            send_ts_us: now_us(),
        };
        let bytes = pkt.encode_to_vec(Some(&payload));
        if let Err(e) = socket.send_to(&bytes, peer).await {
            debug!("blaster download send_to {} failed: {}", peer, e);
            break;
        }
        seq += 1;

        if let Some(d) = inter_packet_delay {
            tokio::time::sleep(d).await;
        } else {
            // Yield occasionally so we don't monopolize the runtime.
            if seq % 256 == 0 {
                tokio::task::yield_now().await;
            }
        }
    }

    // End the session. The client should also send FIN to collect a
    // REPORT, but if it doesn't we'll be reaped by idle eviction.
    debug!("blaster download to {} sent {} packets", peer, seq - 1);
}

/// Convenience entry point used from `main.rs`.
pub async fn run_udp_server(
    addr: impl ToSocketAddrs,
    cancel: CancellationToken,
) -> Result<()> {
    let server = BlasterServer::new(addr).await?;
    server.run(cancel).await
}

// Backwards-compat shim. The Bytes import is unused in the new
// protocol; this re-export keeps old call sites compiling without
// touching the rest of the tree. Remove on the next major.
#[allow(dead_code)]
fn _bytes_marker() -> Bytes {
    Bytes::new()
}
