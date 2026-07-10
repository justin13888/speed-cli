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
//! Mbps before pacing starts to bunch packets. Throughput runs fan out
//! across `parallel_streams` independent sockets (each a separate server
//! session); the target rate is split evenly across them, so more streams
//! raise the aggregate ceiling that pacing granularity would otherwise
//! impose on one socket. For higher rates still, set
//! `target_rate_bps = 0` ("saturate") and let the kernel + scheduler
//! decide.

use bytes::Bytes;
use chrono::Utc;
use colored::Colorize as _;
use eyre::{Result, eyre};
use rand::RngCore;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};
use tracing::{debug, trace};

use super::batch::{BatchIo, MAX_OFFLOAD_DATAGRAM, split_datagrams};
use super::protocol::{
    BlasterPacket, DATA_HEADER_LEN, DataBatchWriter, DataPacketWriter, Mode, ReceiveStats, now_us,
};
use super::tune_socket_buffers;
use crate::{
    TestType,
    performance::engine::{
        LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        measurement_duration_us, offset_us, sample_is_warmup,
    },
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, PeerIdentity,
        Sample, StreamSamples, TestReport, ThroughputResult, UdpRunStats, UdpStatsSide,
        UdpTestConfig,
    },
    utils::format::format_bytes,
};

/// Probe cadence for the latency-under-load stress test (~200 Hz). Tight enough
/// to catch the brief spikes a WiFi card / AP throws under load; `LatencyOnly`
/// keeps the gentler 100 ms cadence.
const STRESS_PROBE_INTERVAL: Duration = Duration::from_millis(5);

/// Idle/loaded split for the latency-under-load test: a short idle baseline,
/// then the remainder under load. The baseline is a third of the run, clamped
/// to `[1s, 5s]` and never longer than the whole test.
fn baseline_duration(total: Duration) -> Duration {
    (total / 3)
        .clamp(Duration::from_secs(1), Duration::from_secs(5))
        .min(total)
}

pub async fn run_udp_client(config: UdpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);
    tracing::info!(
        "{}",
        format!("Starting UDP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

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
    let socket_buffer = config.socket_buffer.unwrap_or(super::SOCKET_BUFFER_BYTES);

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = run_latency(
                &server_addr,
                duration,
                warmup,
                Duration::from_millis(100),
                socket_buffer,
                "latency",
            )
            .await?;
        }
        TestType::LatencyUnderLoad => {
            let baseline = baseline_duration(duration);
            let load = duration
                .saturating_sub(baseline)
                .max(Duration::from_secs(1));
            let payload = config.payload_sizes.iter().copied().next().unwrap_or(1200);
            tracing::info!(
                "WiFi latency stress: {baseline:?} idle baseline, then {load:?} under saturating load"
            );

            // Idle baseline first: probe at the stress cadence with no load.
            result.latency = run_latency(
                &server_addr,
                baseline,
                Duration::ZERO,
                STRESS_PROBE_INTERVAL,
                socket_buffer,
                "idle baseline",
            )
            .await?;

            // Then probe latency while saturating the link in both directions.
            // The load generators run silently so the latency bar stays legible.
            let (loaded, dl, ul) = tokio::join!(
                run_latency(
                    &server_addr,
                    load,
                    Duration::ZERO,
                    STRESS_PROBE_INTERVAL,
                    socket_buffer,
                    "under load",
                ),
                run_download(
                    &server_addr,
                    payload,
                    load,
                    Duration::ZERO,
                    target_rate,
                    config.parallel_streams,
                    socket_buffer,
                    false
                ),
                run_upload(
                    &server_addr,
                    payload,
                    load,
                    Duration::ZERO,
                    target_rate,
                    config.parallel_streams,
                    socket_buffer,
                    false
                ),
            );
            result.latency_under_load = loaded?;
            result.download.insert(payload, dl?);
            result.upload.insert(payload, ul?);
        }
        TestType::Download => {
            for sz in &config.payload_sizes {
                result.download.insert(
                    *sz,
                    run_download(
                        &server_addr,
                        *sz,
                        duration,
                        warmup,
                        target_rate,
                        config.parallel_streams,
                        socket_buffer,
                        true,
                    )
                    .await?,
                );
            }
        }
        TestType::Upload => {
            for sz in &config.payload_sizes {
                result.upload.insert(
                    *sz,
                    run_upload(
                        &server_addr,
                        *sz,
                        duration,
                        warmup,
                        target_rate,
                        config.parallel_streams,
                        socket_buffer,
                        true,
                    )
                    .await?,
                );
            }
        }
        TestType::Bidirectional => {
            for sz in &config.payload_sizes {
                let dl = run_download(
                    &server_addr,
                    *sz,
                    duration,
                    warmup,
                    target_rate,
                    config.parallel_streams,
                    socket_buffer,
                    true,
                )
                .await?;
                let ul = run_upload(
                    &server_addr,
                    *sz,
                    duration,
                    warmup,
                    target_rate,
                    config.parallel_streams,
                    socket_buffer,
                    true,
                )
                .await?;
                result.download.insert(*sz, dl);
                result.upload.insert(*sz, ul);
            }
        }
        TestType::Simultaneous => {
            for sz in &config.payload_sizes {
                let (dl, ul) = tokio::join!(
                    run_download(
                        &server_addr,
                        *sz,
                        duration,
                        warmup,
                        target_rate,
                        config.parallel_streams,
                        socket_buffer,
                        true
                    ),
                    run_upload(
                        &server_addr,
                        *sz,
                        duration,
                        warmup,
                        target_rate,
                        config.parallel_streams,
                        socket_buffer,
                        true
                    ),
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
    probe_interval: Duration,
    socket_buffer: usize,
    label: &str,
) -> Result<Option<LatencyResult>> {
    tracing::info!("Measuring UDP latency ({label}) for {duration:?}...");
    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(server_addr).await?;
    tune_socket_buffers(&socket, socket_buffer);

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

        sleep(probe_interval).await;
    }

    drop(tx);
    measurements = stats_collector
        .finish(
            progress_bar,
            format!("Latency measurement complete ({label})"),
        )
        .await;

    if measurements.is_empty() {
        return Ok(None);
    }
    Ok(Some(LatencyResult {
        measurements,
        timestamp: Utc::now(),
    }))
}

/// Resolve a `host:port` string to a single socket address. The throughput
/// sockets are left unconnected (so the GSO send path can supply an explicit
/// destination — `sendmsg` with a destination on a *connected* socket fails
/// with `EISCONN`), so we resolve the peer address once up front.
async fn resolve_addr(server_addr: &str) -> Result<SocketAddr> {
    tokio::net::lookup_host(server_addr)
        .await?
        .next()
        .ok_or_else(|| eyre!("UDP: no address for {server_addr}"))
}

/// Helper: send a START packet to `dest` and wait briefly for the server to be
/// ready. The server doesn't ACK START; we just give the kernel a tick to
/// deliver it.
async fn send_start(
    socket: &UdpSocket,
    dest: SocketAddr,
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
    socket.send_to(&p.encode_to_vec(None), dest).await?;
    sleep(Duration::from_millis(20)).await;
    Ok(())
}

/// Split a target rate evenly across `streams` so the aggregate matches
/// the request. `0` means "saturate" and is passed through unchanged.
fn split_rate(total_bps: u64, streams: usize) -> u64 {
    if total_bps == 0 {
        return 0;
    }
    (total_bps / streams.max(1) as u64).max(1)
}

/// Fold a per-session receiver stat into the run aggregate: counts sum,
/// jitter takes the worst stream (conservative), and `observed_by` is
/// kept from the first stream (every stream observes the same side).
fn merge_udp_stats(acc: &mut Option<UdpRunStats>, s: UdpRunStats) {
    match acc {
        None => *acc = Some(s),
        Some(a) => {
            a.received_packets += s.received_packets;
            a.bytes_received += s.bytes_received;
            a.lost_packets += s.lost_packets;
            a.out_of_order += s.out_of_order;
            a.duplicates += s.duplicates;
            a.jitter_us = a.jitter_us.max(s.jitter_us);
        }
    }
}

/// Per-stream task output: this stream's samples plus its per-session
/// receiver stats (`None` when the stream couldn't collect them).
type UdpStreamOutput = (Vec<Sample>, Option<UdpRunStats>);

/// Collect the joined per-stream task results into `StreamSamples` (one
/// per stream, indexed by spawn order) and a merged run-level
/// [`UdpRunStats`]. A stream that errored or panicked contributes an
/// empty `StreamSamples` so stream ids stay stable.
fn collect_udp_streams(
    results: Vec<Result<Result<UdpStreamOutput>, tokio::task::JoinError>>,
    direction: &'static str,
) -> (Vec<StreamSamples>, Option<UdpRunStats>) {
    let mut streams = Vec::with_capacity(results.len());
    let mut agg: Option<UdpRunStats> = None;
    for (idx, res) in results.into_iter().enumerate() {
        let stream_id = idx as u32;
        match res {
            Ok(Ok((samples, stats))) => {
                let start_offset_us = samples.first().map(|s| s.t_start_us).unwrap_or(0);
                streams.push(StreamSamples {
                    stream_id,
                    start_offset_us,
                    samples,
                });
                if let Some(s) = stats {
                    merge_udp_stats(&mut agg, s);
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("UDP {direction} stream {idx} failed: {e}");
                streams.push(StreamSamples {
                    stream_id,
                    start_offset_us: 0,
                    samples: Vec::new(),
                });
            }
            Err(e) => {
                tracing::error!("UDP {direction} stream {idx} panicked or was cancelled: {e}");
                streams.push(StreamSamples {
                    stream_id,
                    start_offset_us: 0,
                    samples: Vec::new(),
                });
            }
        }
    }
    (streams, agg)
}

#[allow(clippy::too_many_arguments)]
async fn run_download(
    server_addr: &str,
    payload_size: usize,
    duration: Duration,
    warmup: Duration,
    target_rate_bps: u64,
    parallel_streams: usize,
    socket_buffer: usize,
    show_progress: bool,
) -> Result<ThroughputResult> {
    let parallel_streams = parallel_streams.max(1);
    tracing::info!(
        "UDP download: {} payload, {} target rate, {} stream(s)",
        format_bytes(payload_size).yellow(),
        if target_rate_bps == 0 {
            "saturate".to_string()
        } else {
            format!("{} bps", target_rate_bps)
        }
        .yellow(),
        parallel_streams.to_string().yellow(),
    );
    let progress_bar = if show_progress {
        create_progress_bar(ProgressBarType::Download, duration)
    } else {
        indicatif::ProgressBar::hidden()
    };

    // One shared clock and one shared stats collector so all streams land on
    // the same time axis and the progress bar shows aggregate throughput.
    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    // Each stream is an independent socket (unique source port ⇒ a separate
    // server session) paced at its share of the target rate.
    let per_stream_rate = split_rate(target_rate_bps, parallel_streams);
    let server = resolve_addr(server_addr).await?;

    let mut tasks = Vec::with_capacity(parallel_streams);
    for _ in 0..parallel_streams {
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            tune_socket_buffers(&socket, socket_buffer);
            // GRO: the kernel coalesces several arriving datagrams into one
            // recv, slashing per-packet syscall overhead. The socket only ever
            // *sends* small control packets here, so the don't-fragment bit
            // quinn-udp sets is harmless.
            let batch = BatchIo::new(&socket)?;
            send_start(
                &socket,
                server,
                Mode::Download,
                per_stream_rate,
                payload_size as u32,
                duration,
            )
            .await?;

            // Big enough to hold one GRO batch (up to 64 KiB) or a single
            // oversized (fragmented) datagram, whichever is larger.
            let mut buf = vec![0u8; (payload_size + DATA_HEADER_LEN).max(u16::MAX as usize)];
            let mut samples: Vec<Sample> = Vec::new();
            let mut rx_stats = ReceiveStats::default();
            // A transient recv error (e.g. an ICMP port-unreachable surfaced on
            // the socket) shouldn't abort the whole download; bail only after
            // several in a row, which signals the socket is genuinely dead.
            let mut consecutive_errors = 0u32;
            const MAX_CONSECUTIVE_RECV_ERRORS: u32 = 8;

            while start_time.elapsed() < duration {
                let recv_start = Instant::now();
                let t_start_us = offset_us(start_time, recv_start);
                match timeout(
                    Duration::from_millis(200),
                    batch.recv_coalesced(&socket, &mut buf),
                )
                .await
                {
                    Ok(Ok((len, stride, _src))) => {
                        consecutive_errors = 0;
                        let recv_ts = now_us();
                        let duration_us = recv_start.elapsed().as_micros() as u64;
                        let is_warmup =
                            sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
                        // A GRO buffer may carry several datagrams back to back.
                        for dgram in split_datagrams(&buf[..len], stride) {
                            if let Some((
                                BlasterPacket::Data { seq, send_ts_us },
                                payload_len,
                            )) = BlasterPacket::decode(dgram)
                            {
                                rx_stats.record(seq, payload_len as u64, send_ts_us, recv_ts);
                                let s = Sample::success(
                                    t_start_us,
                                    duration_us,
                                    payload_len as u64,
                                    is_warmup,
                                );
                                samples.push(s.clone());
                                let _ = tx.send(s);
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        let duration_us = recv_start.elapsed().as_micros() as u64;
                        let is_warmup =
                            sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
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
                                "UDP download stream: {consecutive_errors} consecutive recv errors, stopping"
                            );
                            break;
                        }
                    }
                    Err(_) => continue,
                }
            }

            let _ = socket
                .send_to(&BlasterPacket::Fin.encode_to_vec(None), server)
                .await;

            let stats = UdpRunStats {
                observed_by: UdpStatsSide::Local,
                received_packets: rx_stats.received,
                bytes_received: rx_stats.bytes_received,
                lost_packets: rx_stats.lost(),
                out_of_order: rx_stats.out_of_order,
                duplicates: rx_stats.duplicates,
                jitter_us: rx_stats.jitter_us(),
            };
            Ok::<UdpStreamOutput, eyre::Report>((samples, Some(stats)))
        }));
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;
    let end_time = Instant::now();

    let (streams, udp_stats) = collect_udp_streams(results, "download");
    if let Some(s) = &udp_stats {
        debug!(
            "UDP download complete: {} packets, {} bytes, {} lost, jitter {} us ({} streams)",
            s.received_packets,
            s.bytes_received,
            s.lost_packets,
            s.jitter_us,
            streams.len()
        );
    }

    Ok(ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start_time, end_time, warmup),
        timestamp: Utc::now(),
        udp_stats,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_upload(
    server_addr: &str,
    payload_size: usize,
    duration: Duration,
    warmup: Duration,
    target_rate_bps: u64,
    parallel_streams: usize,
    socket_buffer: usize,
    show_progress: bool,
) -> Result<ThroughputResult> {
    let parallel_streams = parallel_streams.max(1);
    tracing::info!(
        "UDP upload: {} payload, {} target rate, {} stream(s)",
        format_bytes(payload_size).yellow(),
        if target_rate_bps == 0 {
            "saturate".to_string()
        } else {
            format!("{} bps", target_rate_bps)
        }
        .yellow(),
        parallel_streams.to_string().yellow(),
    );
    let progress_bar = if show_progress {
        create_progress_bar(ProgressBarType::Upload, duration)
    } else {
        indicatif::ProgressBar::hidden()
    };

    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    // Each stream is its own socket (a separate server session), paced at its
    // share of the target rate so the aggregate matches the request.
    let per_stream_rate = split_rate(target_rate_bps, parallel_streams);
    let server = resolve_addr(server_addr).await?;

    let mut tasks = Vec::with_capacity(parallel_streams);
    for _ in 0..parallel_streams {
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            tune_socket_buffers(&socket, socket_buffer);
            send_start(
                &socket,
                server,
                Mode::Upload,
                per_stream_rate,
                payload_size as u32,
                duration,
            )
            .await?;

            let mut payload = vec![0u8; payload_size];
            rand::rng().fill_bytes(&mut payload);

            let inter_packet_delay = if per_stream_rate > 0 {
                let bps = per_stream_rate as f64 / 8.0;
                Some(Duration::from_secs_f64(
                    (payload_size as f64) / bps.max(1.0),
                ))
            } else {
                None
            };

            let mut samples: Vec<Sample> = Vec::new();

            let seg_len = DATA_HEADER_LEN + payload_size;
            if seg_len <= MAX_OFFLOAD_DATAGRAM {
                // GSO path: frame several datagrams contiguously and hand the
                // whole buffer to the kernel in one syscall. Only taken for
                // MTU-sized datagrams, so the don't-fragment bit quinn-udp sets
                // never bites. send_segmented awaits writability internally, so
                // it provides natural backpressure at saturation.
                let batch = BatchIo::new(&socket)?;
                let segments = batch.segments_for(seg_len);
                let mut writer = DataBatchWriter::new(&payload, segments);
                let seg_size = writer.segment_size();
                let batch_bytes = (segments * payload_size) as u64;
                let mut seq: u64 = 1;
                while start_time.elapsed() < duration {
                    let send_instant = Instant::now();
                    let t_start_us = offset_us(start_time, send_instant);
                    let bytes = writer.frame_batch(seq, now_us());
                    match batch.send_segmented(&socket, server, bytes, seg_size).await {
                        Ok(()) => {
                            let duration_us = send_instant.elapsed().as_micros() as u64;
                            let is_warmup =
                                sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
                            // One sample per batch, carrying the batch's bytes.
                            let s =
                                Sample::success(t_start_us, duration_us, batch_bytes, is_warmup);
                            samples.push(s.clone());
                            let _ = tx.send(s);
                            seq += segments as u64;
                        }
                        Err(e) => {
                            // send_segmented only surfaces errors when the socket
                            // itself is unusable (it retries WouldBlock and
                            // swallows transient errnos); record and back off.
                            let duration_us = send_instant.elapsed().as_micros() as u64;
                            let is_warmup =
                                sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
                            let s = Sample::failure(
                                t_start_us,
                                duration_us,
                                ConnectionError::TransferFailed(format!("UDP GSO send: {e}")),
                                0,
                                is_warmup,
                            );
                            samples.push(s.clone());
                            let _ = tx.send(s);
                            tokio::task::yield_now().await;
                        }
                    }
                    if let Some(d) = inter_packet_delay {
                        sleep(d.saturating_mul(segments as u32)).await;
                    }
                }
            } else {
                // Plain per-packet path for datagrams larger than the MTU: they
                // require IP fragmentation, so this socket is left without the
                // quinn-udp PMTU/don't-fragment setup.
                let mut packet = DataPacketWriter::new(&payload);
                let mut seq: u64 = 1;
                while start_time.elapsed() < duration {
                    let send_instant = Instant::now();
                    let t_start_us = offset_us(start_time, send_instant);
                    let bytes = packet.frame(seq, now_us());
                    match socket.send_to(bytes, server).await {
                        Ok(_) => {
                            let duration_us = send_instant.elapsed().as_micros() as u64;
                            let is_warmup =
                                sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
                            let s = Sample::success(
                                t_start_us,
                                duration_us,
                                payload_size as u64,
                                is_warmup,
                            );
                            samples.push(s.clone());
                            let _ = tx.send(s);
                            seq += 1;
                        }
                        Err(e) => {
                            // UDP send failures are *expected* at saturation:
                            // ENOBUFS (kernel send queue full) and EAGAIN/WouldBlock
                            // both mean "back off, try again", not "test is over".
                            // We record the failed attempt for accounting and yield
                            // so the kernel can drain. Any other errno (host
                            // unreachable, etc.) we record and keep going too.
                            let kind = e.kind();
                            let duration_us = send_instant.elapsed().as_micros() as u64;
                            let is_warmup =
                                sample_is_warmup(t_start_us.saturating_add(duration_us), warmup);
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
            }

            // FIN + REPORT collection for this stream's session.
            let mut report: Option<BlasterPacket> = None;
            for _ in 0..5 {
                let _ = socket
                    .send_to(&BlasterPacket::Fin.encode_to_vec(None), server)
                    .await;
                let mut buf = vec![0u8; 4096];
                if let Ok(Ok((n, _))) =
                    timeout(Duration::from_millis(200), socket.recv_from(&mut buf)).await
                    && let Some((p, _)) = BlasterPacket::decode(&buf[..n])
                    && matches!(p, BlasterPacket::Report { .. })
                {
                    report = Some(p);
                    break;
                }
            }
            let stats = if let Some(BlasterPacket::Report {
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
                    "UDP upload stream: no REPORT received from server; loss/jitter unreported"
                );
                None
            };
            Ok::<UdpStreamOutput, eyre::Report>((samples, stats))
        }));
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;
    let end_time = Instant::now();

    let (streams, udp_stats) = collect_udp_streams(results, "upload");

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
