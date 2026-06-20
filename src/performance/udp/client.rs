//! UDP blaster client.
//!
//! Three modes:
//! - **Latency**: send PING / wait for PONG, measure round-trip time.
//! - **Download**: ask the server to send to us at `target_rate_bps`
//!   for `duration`, count what we got. Loss / OOO / jitter are
//!   computed locally from the DATA stream.
//! - **Upload**: send to the server at the target rate, then ask for a
//!   REPORT to see how many packets actually arrived.
//!
//! Pacing uses `tokio::time::sleep`, which has roughly 1 ms
//! resolution. That caps the achievable target rate at a few hundred
//! Mbps before pacing starts to bunch packets. For higher rates, set
//! `target_rate_bps = 0` ("saturate") and let the kernel + scheduler
//! decide.

use bytes::Bytes;
use chrono::Utc;
use colored::Colorize as _;
use eyre::{Result, eyre};
use rand::RngCore;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};
use tracing::{debug, trace};

use super::protocol::{BlasterPacket, Mode, ReceiveStats, now_us};
use crate::{
    TestType,
    performance::engine::{
        LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        measurement_duration_us, offset_us,
    },
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, PeerIdentity,
        Sample, StreamSamples, TestReport, ThroughputResult, UdpRunStats, UdpStatsSide,
        UdpTestConfig,
    },
    utils::format::format_bytes,
};

pub async fn run_udp_client(config: UdpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);
    tracing::info!(
        "{}",
        format!("Starting UDP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

    if config.parallel_streams > 1 {
        tracing::warn!(
            "UDP test was asked for {} parallel streams, but the blaster runs a single \
             stream per session. Multi-stream UDP is tracked in the Phase 3 follow-up.",
            config.parallel_streams
        );
    }

    // Pre-flight: PING/PONG with a 1s timeout. Capture the socket's
    // local and remote addresses for the report's peers section, then
    // run the identity handshake on the same socket so the report can
    // record the peer's view of us.
    let (preflight_local, preflight_peer, server_hello) = {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(&server_addr).await?;
        let local_addr = socket.local_addr().ok();
        let peer_addr = socket.peer_addr().ok();
        let p = BlasterPacket::Ping {
            send_ts_us: now_us(),
        };
        socket.send(&p.encode_to_vec(None)).await?;
        let mut buf = vec![0u8; 4096];
        match timeout(Duration::from_secs(1), socket.recv(&mut buf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                return Err(eyre!(
                    "UDP pre-flight recv from {} failed: {}",
                    server_addr,
                    e
                ));
            }
            Err(_) => {
                return Err(eyre!(
                    "UDP pre-flight: no response from {} within 1s",
                    server_addr
                ));
            }
        }

        // Identity handshake. Best-effort: any failure leaves the
        // server side of `peers` empty, but the test still runs.
        let server_hello = run_udp_hello(&socket).await;

        (local_addr, peer_addr, server_hello)
    };

    let start_time = Utc::now();
    let mut result = NetworkTestResult::new_udp().with_accounting(config.accounting);
    let duration = Duration::from_secs(config.duration);
    let warmup = config.warmup;
    let target_rate = config.target_rate_bps;

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = run_latency(&server_addr, duration, warmup).await?;
        }
        TestType::Download => {
            for sz in &config.payload_sizes {
                result.download.insert(
                    *sz,
                    run_download(&server_addr, *sz, duration, warmup, target_rate).await?,
                );
            }
        }
        TestType::Upload => {
            for sz in &config.payload_sizes {
                result.upload.insert(
                    *sz,
                    run_upload(&server_addr, *sz, duration, warmup, target_rate).await?,
                );
            }
        }
        TestType::Bidirectional => {
            for sz in &config.payload_sizes {
                let dl = run_download(&server_addr, *sz, duration, warmup, target_rate).await?;
                let ul = run_upload(&server_addr, *sz, duration, warmup, target_rate).await?;
                result.download.insert(*sz, dl);
                result.upload.insert(*sz, ul);
            }
        }
        TestType::Simultaneous => {
            for sz in &config.payload_sizes {
                let (dl, ul) = tokio::join!(
                    run_download(&server_addr, *sz, duration, warmup, target_rate),
                    run_upload(&server_addr, *sz, duration, warmup, target_rate),
                );
                result.download.insert(*sz, dl?);
                result.upload.insert(*sz, ul?);
            }
        }
        TestType::FullDuplex => {
            return Err(eyre!(
                "FullDuplex test type is TCP-only. Use --type=simultaneous for UDP."
            ));
        }
    }

    let mut report: TestReport = (start_time, config, result).into();
    report.peers.client.local_addr = preflight_local;
    report.peers.client.remote_addr = preflight_peer;
    if let Some(h) = server_hello {
        report.peers.server.identity = Some(h.0);
        report.peers.server.observed_client_addr = Some(h.1);
    }
    Ok(report)
}

/// Send a `Hello` and await `HelloAck` on the given (already-connected)
/// UDP socket. Returns `(server_identity, observed_client_addr)` on
/// success, or `None` if the server didn't reply with a parseable
/// `HelloAck` within a short window.
async fn run_udp_hello(socket: &UdpSocket) -> Option<(PeerIdentity, std::net::SocketAddr)> {
    let mut id_buf = Vec::new();
    ciborium::into_writer(&PeerIdentity::local(), &mut id_buf).ok()?;
    let hello = BlasterPacket::Hello {
        identity_cbor: id_buf,
        t_send_us: now_us(),
    };
    socket.send(&hello.encode_to_vec(None)).await.ok()?;

    let mut buf = vec![0u8; 8192];
    let n = match timeout(Duration::from_millis(500), socket.recv(&mut buf)).await {
        Ok(Ok(n)) => n,
        _ => return None,
    };
    let (decoded, _) = BlasterPacket::decode(&buf[..n])?;
    let BlasterPacket::HelloAck {
        identity_cbor,
        observed_client_addr_cbor,
        ..
    } = decoded
    else {
        return None;
    };
    let identity: PeerIdentity = ciborium::from_reader(identity_cbor.as_slice()).ok()?;
    let addr: std::net::SocketAddr =
        ciborium::from_reader(observed_client_addr_cbor.as_slice()).ok()?;
    Some((identity, addr))
}

async fn run_latency(
    server_addr: &str,
    duration: Duration,
    warmup: Duration,
) -> Result<Option<LatencyResult>> {
    tracing::info!("Measuring UDP latency for {duration:?}...");
    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(server_addr).await?;

    let start = Instant::now();
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);
    let mut buf = [0u8; 4096];
    let mut measurements = Vec::new();

    while start.elapsed() < duration {
        let in_warmup = start.elapsed() < warmup;
        let probe_start = Instant::now();
        let t_start_us = offset_us(start, probe_start);

        let p = BlasterPacket::Ping {
            send_ts_us: now_us(),
        };
        let m = match socket.send(&p.encode_to_vec(None)).await {
            Ok(_) => match timeout(Duration::from_secs(1), socket.recv(&mut buf)).await {
                Ok(Ok(n)) => match BlasterPacket::decode(&buf[..n]) {
                    Some((BlasterPacket::Pong { .. }, _)) => LatencyMeasurement::success(
                        t_start_us,
                        probe_start.elapsed().as_micros() as u64,
                    ),
                    _ => LatencyMeasurement::dropped(t_start_us),
                },
                _ => LatencyMeasurement::dropped(t_start_us),
            },
            Err(e) => {
                trace!("UDP send error: {e}");
                LatencyMeasurement::dropped(t_start_us)
            }
        };

        if !in_warmup {
            measurements.push(m.clone());
            let _ = tx.send(m);
        }

        sleep(Duration::from_millis(100)).await;
    }

    drop(tx);
    measurements = stats_collector
        .finish(progress_bar, "Latency measurement complete".to_string())
        .await;

    if measurements.is_empty() {
        return Ok(None);
    }
    Ok(Some(LatencyResult {
        measurements,
        timestamp: Utc::now(),
    }))
}

/// Helper: send a START packet and wait briefly for the server to be
/// ready. The server doesn't ACK START; we just give the kernel a tick
/// to deliver it.
async fn send_start(
    socket: &UdpSocket,
    mode: Mode,
    target_rate_bps: u64,
    payload_size: u32,
    duration: Duration,
) -> Result<()> {
    let p = BlasterPacket::Start {
        mode,
        target_rate_bps,
        payload_size,
        duration_ms: duration.as_millis() as u64,
    };
    socket.send(&p.encode_to_vec(None)).await?;
    sleep(Duration::from_millis(20)).await;
    Ok(())
}

async fn run_download(
    server_addr: &str,
    payload_size: usize,
    duration: Duration,
    warmup: Duration,
    target_rate_bps: u64,
) -> Result<ThroughputResult> {
    tracing::info!(
        "UDP download: {} payload, {} target rate",
        format_bytes(payload_size).yellow(),
        if target_rate_bps == 0 {
            "saturate".to_string()
        } else {
            format!("{} bps", target_rate_bps)
        }
        .yellow()
    );
    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(server_addr).await?;

    send_start(
        &socket,
        Mode::Download,
        target_rate_bps,
        payload_size as u32,
        duration,
    )
    .await?;

    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);
    let mut buf = vec![0u8; payload_size + 64];
    let mut samples: Vec<Sample> = Vec::new();
    let mut rx_stats = ReceiveStats::default();
    // A transient recv error (e.g. an ICMP port-unreachable surfaced on the
    // socket) shouldn't abort the whole download; bail only after several in a
    // row, which signals the socket is genuinely dead.
    let mut consecutive_errors = 0u32;
    const MAX_CONSECUTIVE_RECV_ERRORS: u32 = 8;

    while start_time.elapsed() < duration {
        let is_warmup = start_time.elapsed() < warmup;
        let recv_start = Instant::now();
        let t_start_us = offset_us(start_time, recv_start);
        match timeout(Duration::from_millis(200), socket.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                consecutive_errors = 0;
                let recv_ts = now_us();
                if let Some((BlasterPacket::Data { seq, send_ts_us }, payload_len)) =
                    BlasterPacket::decode(&buf[..n])
                {
                    rx_stats.record(seq, payload_len as u64, send_ts_us, recv_ts);
                    let duration_us = recv_start.elapsed().as_micros() as u64;
                    let s = Sample::success(t_start_us, duration_us, payload_len as u64, is_warmup);
                    samples.push(s.clone());
                    let _ = tx.send(s);
                }
            }
            Ok(Err(e)) => {
                let duration_us = recv_start.elapsed().as_micros() as u64;
                let s = Sample::failure(
                    t_start_us,
                    duration_us,
                    ConnectionError::Unknown(format!("UDP recv error: {e}")),
                    0,
                    is_warmup,
                );
                samples.push(s.clone());
                let _ = tx.send(s);
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_RECV_ERRORS {
                    tracing::warn!(
                        "UDP download: {consecutive_errors} consecutive recv errors, stopping"
                    );
                    break;
                }
            }
            Err(_) => continue,
        }
    }

    let _ = socket.send(&BlasterPacket::Fin.encode_to_vec(None)).await;

    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;

    let end_time = Instant::now();
    debug!(
        "UDP download complete: {} packets, {} bytes, {} lost, jitter {} us",
        rx_stats.received,
        rx_stats.bytes_received,
        rx_stats.lost(),
        rx_stats.jitter_us()
    );

    let start_offset_us = samples.first().map(|s| s.t_start_us).unwrap_or(0);
    let streams = vec![StreamSamples {
        stream_id: 0,
        start_offset_us,
        samples,
    }];
    let udp_stats = Some(UdpRunStats {
        observed_by: UdpStatsSide::Local,
        received_packets: rx_stats.received,
        bytes_received: rx_stats.bytes_received,
        lost_packets: rx_stats.lost(),
        out_of_order: rx_stats.out_of_order,
        duplicates: rx_stats.duplicates,
        jitter_us: rx_stats.jitter_us(),
    });
    Ok(ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start_time, end_time, warmup),
        timestamp: Utc::now(),
        udp_stats,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    })
}

async fn run_upload(
    server_addr: &str,
    payload_size: usize,
    duration: Duration,
    warmup: Duration,
    target_rate_bps: u64,
) -> Result<ThroughputResult> {
    tracing::info!(
        "UDP upload: {} payload, {} target rate",
        format_bytes(payload_size).yellow(),
        if target_rate_bps == 0 {
            "saturate".to_string()
        } else {
            format!("{} bps", target_rate_bps)
        }
        .yellow()
    );
    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(server_addr).await?;

    send_start(
        &socket,
        Mode::Upload,
        target_rate_bps,
        payload_size as u32,
        duration,
    )
    .await?;

    let mut payload = vec![0u8; payload_size];
    rand::rng().fill_bytes(&mut payload);

    let inter_packet_delay = if target_rate_bps > 0 {
        let bps = target_rate_bps as f64 / 8.0;
        Some(Duration::from_secs_f64(
            (payload_size as f64) / bps.max(1.0),
        ))
    } else {
        None
    };

    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);
    let mut samples: Vec<Sample> = Vec::new();
    let mut seq: u64 = 1;

    while start_time.elapsed() < duration {
        let is_warmup = start_time.elapsed() < warmup;
        let send_start = Instant::now();
        let t_start_us = offset_us(start_time, send_start);
        let p = BlasterPacket::Data {
            seq,
            send_ts_us: now_us(),
        };
        let bytes = p.encode_to_vec(Some(&payload));
        match socket.send(&bytes).await {
            Ok(_) => {
                let duration_us = send_start.elapsed().as_micros() as u64;
                let s = Sample::success(t_start_us, duration_us, payload_size as u64, is_warmup);
                samples.push(s.clone());
                let _ = tx.send(s);
                seq += 1;
            }
            Err(e) => {
                // UDP send failures are *expected* at saturation: ENOBUFS
                // (kernel send queue full) and EAGAIN/WouldBlock both mean
                // "back off, try again", not "test is over". We record the
                // failed attempt for accounting and yield so the kernel can
                // drain. Any other errno (host unreachable, etc.) we record
                // and keep going too.
                let kind = e.kind();
                let duration_us = send_start.elapsed().as_micros() as u64;
                let s = Sample::failure(
                    t_start_us,
                    duration_us,
                    ConnectionError::TransferFailed(format!("UDP send error: {e}")),
                    0,
                    is_warmup,
                );
                samples.push(s.clone());
                let _ = tx.send(s);
                if matches!(
                    kind,
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::OutOfMemory
                ) {
                    tokio::task::yield_now().await;
                } else {
                    sleep(Duration::from_millis(1)).await;
                }
                // Note: we deliberately do *not* increment `seq` here -
                // the packet was never put on the wire, so the receiver
                // shouldn't count it as a gap.
            }
        }

        if let Some(d) = inter_packet_delay {
            sleep(d).await;
        } else if seq.is_multiple_of(256) {
            tokio::task::yield_now().await;
        }
    }

    // FIN + REPORT collection.
    let mut report: Option<BlasterPacket> = None;
    for _ in 0..5 {
        let _ = socket.send(&BlasterPacket::Fin.encode_to_vec(None)).await;
        let mut buf = vec![0u8; 4096];
        if let Ok(Ok(n)) = timeout(Duration::from_millis(200), socket.recv(&mut buf)).await
            && let Some((p, _)) = BlasterPacket::decode(&buf[..n])
            && matches!(p, BlasterPacket::Report { .. })
        {
            report = Some(p);
            break;
        }
    }
    let udp_stats = if let Some(BlasterPacket::Report {
        received,
        bytes_received,
        lost,
        out_of_order,
        jitter_us,
        duplicates,
    }) = report
    {
        debug!(
            "server REPORT: received={} bytes={} lost={} oos={} dups={} jitter={}us",
            received, bytes_received, lost, out_of_order, duplicates, jitter_us
        );
        Some(UdpRunStats {
            observed_by: UdpStatsSide::Remote,
            received_packets: received,
            bytes_received,
            lost_packets: lost,
            out_of_order,
            duplicates,
            jitter_us,
        })
    } else {
        tracing::warn!(
            "UDP upload: no REPORT received from server; loss/jitter will be unreported"
        );
        None
    };

    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;

    let end_time = Instant::now();
    let start_offset_us = samples.first().map(|s| s.t_start_us).unwrap_or(0);
    let streams = vec![StreamSamples {
        stream_id: 0,
        start_offset_us,
        samples,
    }];
    Ok(ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start_time, end_time, warmup),
        timestamp: Utc::now(),
        udp_stats,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    })
}

// Bytes import is used implicitly through the protocol module but the
// compiler still needs it for any future ad-hoc use; keep the import
// from going unused. Tiny no-op.
#[allow(dead_code)]
fn _b() -> Bytes {
    Bytes::new()
}
