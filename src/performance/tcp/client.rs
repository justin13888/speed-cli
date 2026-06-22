use chrono::Utc;
use colored::Colorize as _;
use eyre::Result;

use rand::{prelude::*, rng};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    performance::engine::{
        LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        measurement_duration_us, offset_us,
    },
    performance::tcp::handshake::client_hello,
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, PeerIdentity,
        Sample, StreamSamples, TcpTestConfig, TestReport, ThroughputResult,
    },
    utils::format::format_bytes,
};

/// Run a full-duplex test on `parallel_connections` TCP connections,
/// each simultaneously reading and writing for `duration`. Returns
/// (download_result, upload_result) where each direction's stats come
/// from the same set of connections (so they're directly comparable -
/// asymmetric throughput here is the actual link asymmetry under load).
#[allow(clippy::too_many_arguments)]
async fn run_full_duplex_test(
    server: &str,
    port: u16,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
    read_buffer_size: usize,
    warmup: Duration,
) -> Result<(ThroughputResult, ThroughputResult)> {
    tracing::info!(
        "Starting TCP full-duplex test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let dl_pb = create_progress_bar(ProgressBarType::Download, duration);
    let ul_pb = create_progress_bar(ProgressBarType::Upload, duration);
    let start_time = Instant::now();

    let (dl_collector, dl_tx) = ThroughputStatsCollector::new(dl_pb.clone(), start_time, duration);
    let (ul_collector, ul_tx) = ThroughputStatsCollector::new(ul_pb.clone(), start_time, duration);

    let mut tasks: Vec<tokio::task::JoinHandle<(Vec<Sample>, Vec<Sample>)>> =
        Vec::with_capacity(parallel_connections);

    for i in 0..parallel_connections {
        let server = server.to_string();
        let dl_tx = dl_tx.clone();
        let ul_tx = ul_tx.clone();

        let task = tokio::spawn(async move {
            let mut dl_local: Vec<Sample> = Vec::new();
            let mut ul_local: Vec<Sample> = Vec::new();
            let addr = format!("{server}:{port}");

            let stream = match TcpStream::connect(&addr).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("TCP full-duplex connect error on conn {i}: {e}");
                    return (dl_local, ul_local);
                }
            };
            if let Err(e) = stream.set_nodelay(true) {
                tracing::debug!("TCP set_nodelay failed on full-duplex conn {i}: {e}");
            }
            let (mut read_half, mut write_half) = stream.into_split();

            // Send the F command to put the server into full-duplex mode.
            if let Err(e) = write_half.write_all(b"F").await {
                tracing::warn!("Failed to send F command on conn {i}: {e}");
                return (dl_local, ul_local);
            }

            // Random upload data.
            let upload_data = {
                let mut data = vec![0u8; payload_size];
                rng().fill_bytes(&mut data);
                data
            };

            let mut read_buf = vec![0u8; read_buffer_size];

            let read_fut = async {
                while start_time.elapsed() < duration {
                    let read_start = Instant::now();
                    let t_start_us = offset_us(start_time, read_start);
                    let is_warmup = start_time.elapsed() < warmup;
                    match read_half.read(&mut read_buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let duration_us = read_start.elapsed().as_micros() as u64;
                            let s = Sample::success(t_start_us, duration_us, n as u64, is_warmup);
                            dl_local.push(s.clone());
                            let _ = dl_tx.send(s);
                        }
                        Err(e) => {
                            let duration_us = read_start.elapsed().as_micros() as u64;
                            let s = Sample::failure(
                                t_start_us,
                                duration_us,
                                ConnectionError::Unknown(e.to_string()),
                                0,
                                is_warmup,
                            );
                            dl_local.push(s.clone());
                            let _ = dl_tx.send(s);
                            break;
                        }
                    }
                }
            };

            let write_fut = async {
                while start_time.elapsed() < duration {
                    let write_start = Instant::now();
                    let t_start_us = offset_us(start_time, write_start);
                    let is_warmup = start_time.elapsed() < warmup;
                    match write_half.write_all(&upload_data).await {
                        Ok(()) => {
                            let duration_us = write_start.elapsed().as_micros() as u64;
                            let s = Sample::success(
                                t_start_us,
                                duration_us,
                                upload_data.len() as u64,
                                is_warmup,
                            );
                            ul_local.push(s.clone());
                            let _ = ul_tx.send(s);
                        }
                        Err(e) => {
                            let duration_us = write_start.elapsed().as_micros() as u64;
                            let s = Sample::failure(
                                t_start_us,
                                duration_us,
                                ConnectionError::Unknown(format!(
                                    "TCP full-duplex write error on conn {i}: {e}"
                                )),
                                0,
                                is_warmup,
                            );
                            ul_local.push(s.clone());
                            let _ = ul_tx.send(s);
                            break;
                        }
                    }
                }
            };

            tokio::join!(read_fut, write_fut);
            (dl_local, ul_local)
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;
    drop(dl_tx);
    drop(ul_tx);
    let _ = dl_collector
        .finish(dl_pb, "Full-duplex download complete".to_string())
        .await;
    let _ = ul_collector
        .finish(ul_pb, "Full-duplex upload complete".to_string())
        .await;

    let mut dl_streams: Vec<StreamSamples> = Vec::with_capacity(results.len());
    let mut ul_streams: Vec<StreamSamples> = Vec::with_capacity(results.len());
    for (idx, joined) in results.into_iter().enumerate() {
        match joined {
            Ok((dl, ul)) => {
                dl_streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: dl.first().map(|s| s.t_start_us).unwrap_or(0),
                    samples: dl,
                });
                ul_streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: ul.first().map(|s| s.t_start_us).unwrap_or(0),
                    samples: ul,
                });
            }
            Err(e) => {
                tracing::error!("full-duplex task {idx} panicked or was cancelled: {e}");
                dl_streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: 0,
                    samples: Vec::new(),
                });
                ul_streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: 0,
                    samples: Vec::new(),
                });
            }
        }
    }

    let end_time = Instant::now();
    let duration_us = measurement_duration_us(start_time, end_time, warmup);
    let timestamp = chrono::Utc::now();

    Ok((
        ThroughputResult {
            streams: dl_streams,
            total_duration_us: duration_us,
            timestamp,
            udp_stats: None,
            udp_series: Vec::new(),
            udp_series_window_us: 0,
        },
        ThroughputResult {
            streams: ul_streams,
            total_duration_us: duration_us,
            timestamp,
            udp_stats: None,
            udp_series: Vec::new(),
            udp_series_window_us: 0,
        },
    ))
}

/// Take the join_all results from a multi-stream throughput test and
/// produce per-stream samples. Logs any task that panicked rather than
/// failing the whole test.
fn collect_streams(
    results: Vec<Result<Vec<Sample>, tokio::task::JoinError>>,
    direction: &'static str,
) -> Vec<StreamSamples> {
    let mut streams = Vec::with_capacity(results.len());
    for (idx, result) in results.into_iter().enumerate() {
        match result {
            Ok(task_samples) => {
                streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: task_samples.first().map(|s| s.t_start_us).unwrap_or(0),
                    samples: task_samples,
                });
            }
            Err(e) => {
                tracing::error!("{direction} task {idx} panicked or was cancelled: {e}");
                streams.push(StreamSamples {
                    stream_id: idx as u32,
                    start_offset_us: 0,
                    samples: Vec::new(),
                });
            }
        }
    }
    streams
}

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);

    tracing::info!(
        "{}",
        format!("Starting TCP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

    // Pre-flight: confirm we can reach the server before running the full
    // test. Saves the user a 30-second timeout when the server is down.
    // The pre-flight socket also carries the addresses we record on the
    // report's local view, so the report knows which client/server pair
    // it describes.
    let (preflight_local, preflight_peer) = match tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(&server_addr),
    )
    .await
    {
        Ok(Ok(stream)) => (stream.local_addr().ok(), stream.peer_addr().ok()),
        Ok(Err(e)) => {
            return Err(eyre::eyre!(
                "TCP pre-flight connect to {} failed: {}",
                server_addr,
                e
            ));
        }
        Err(_) => {
            return Err(eyre::eyre!(
                "TCP pre-flight connect to {} timed out after 5s",
                server_addr
            ));
        }
    };

    // Identity handshake on a fresh connection. Best-effort — older
    // servers / partial deployments simply return None and we record
    // an empty `RemoteView`.
    let hello = client_hello(&server_addr, &PeerIdentity::local()).await;

    let start_time = Utc::now();

    let mut result = NetworkTestResult::new_tcp().with_accounting(config.accounting);

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = measure_tcp_latency(&config).await?;
        }
        TestType::Download => {
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.read_buffer_size,
                        config.warmup,
                    )
                    .await?,
                );
            }
        }
        TestType::Upload => {
            for payload_size in &config.payload_sizes {
                result.upload.insert(
                    *payload_size,
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.warmup,
                    )
                    .await?,
                );
            }
        }
        TestType::Bidirectional => {
            // Run download and upload sequentially
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.read_buffer_size,
                        config.warmup,
                    )
                    .await?,
                );
                result.upload.insert(
                    *payload_size,
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.warmup,
                    )
                    .await?,
                );
            }
        }
        TestType::Simultaneous => {
            // Run download and upload concurrently
            for payload_size in &config.payload_sizes {
                let (download_result, upload_result) = tokio::join!(
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.read_buffer_size,
                        config.warmup,
                    ),
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                        config.warmup,
                    )
                );

                result.download.insert(*payload_size, download_result?);
                result.upload.insert(*payload_size, upload_result?);
            }
        }
        TestType::FullDuplex => {
            for payload_size in &config.payload_sizes {
                let (down, up) = run_full_duplex_test(
                    &config.server,
                    config.port,
                    config.parallel_connections,
                    *payload_size,
                    config.duration,
                    config.read_buffer_size,
                    config.warmup,
                )
                .await?;
                result.download.insert(*payload_size, down);
                result.upload.insert(*payload_size, up);
            }
        }
    }

    let mut report: TestReport = (start_time, config, result).into();
    report.peers.client.local_addr = preflight_local;
    report.peers.client.remote_addr = preflight_peer;
    if let Some(h) = hello {
        report.peers.server.identity = Some(h.server_identity);
        report.peers.server.observed_client_addr = Some(h.observed_client_addr);
    }
    Ok(report)
}

/// Measure TCP application-level RTT on a *warmed* TCP connection by
/// pinging the server's `'P'` handler. Distinct from connect-time
/// latency: a TCP three-way handshake involves an extra packet round
/// and is heavily influenced by syncookies / accept-queue depth, which
/// is not what most users mean by "TCP latency". This number is
/// directly comparable to UDP and HTTP RTT in this same tool.
async fn measure_tcp_latency(config: &TcpTestConfig) -> Result<Option<LatencyResult>> {
    let addr = format!("{}:{}", config.server, config.port);
    let duration = config.duration;
    let warmup = config.warmup;
    let mut measurements = Vec::new();

    tracing::info!("Measuring TCP in-stream RTT for {duration:?}...");

    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);
    let start = Instant::now();
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

    let mut stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            return Err(eyre::eyre!(
                "TCP latency: connect to {} failed: {}",
                addr,
                e
            ));
        }
    };
    if let Err(e) = stream.set_nodelay(true) {
        tracing::debug!("TCP set_nodelay failed on latency stream: {e}");
    }
    if let Err(e) = stream.write_all(b"P").await {
        return Err(eyre::eyre!(
            "TCP latency: failed to send 'P' command: {}",
            e
        ));
    }
    sleep(Duration::from_millis(10)).await;

    let mut send_buf = [0u8; 8];
    let mut recv_buf = [0u8; 8];
    let probe_interval = Duration::from_millis(10);

    while start.elapsed() < duration {
        let in_warmup = start.elapsed() < warmup;
        let probe_start = Instant::now();
        let t_start_us = offset_us(start, probe_start);
        let nonce: u64 = probe_start.elapsed().as_micros() as u64;
        send_buf.copy_from_slice(&nonce.to_le_bytes());
        let measurement = match stream.write_all(&send_buf).await {
            Ok(()) => {
                match tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut recv_buf))
                    .await
                {
                    Ok(Ok(_)) => {
                        let echoed = u64::from_le_bytes(recv_buf);
                        if echoed != nonce {
                            tracing::debug!("TCP latency: nonce mismatch, discarding sample");
                            LatencyMeasurement::dropped(t_start_us)
                        } else {
                            LatencyMeasurement::success(
                                t_start_us,
                                probe_start.elapsed().as_micros() as u64,
                            )
                        }
                    }
                    Ok(Err(e)) => {
                        trace!("TCP latency read error: {e}");
                        LatencyMeasurement::dropped(t_start_us)
                    }
                    Err(_) => {
                        tracing::debug!("TCP latency: read timeout, dropped sample");
                        LatencyMeasurement::dropped(t_start_us)
                    }
                }
            }
            Err(e) => {
                trace!("TCP latency write error: {e}");
                LatencyMeasurement::dropped(t_start_us)
            }
        };

        if !in_warmup {
            measurements.push(measurement.clone());
            let _ = tx.send(measurement);
        }

        sleep(probe_interval).await;
    }

    let _ = stream.shutdown().await;
    drop(tx);

    measurements = stats_collector
        .finish(progress_bar, "Latency measurement complete".to_string())
        .await;

    if measurements.is_empty() {
        return Ok(None);
    }

    Ok(Some(LatencyResult {
        measurements,
        timestamp: chrono::Utc::now(),
    }))
}

async fn run_download_test(
    server: &str,
    port: u16,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
    read_buffer_size: usize,
    warmup: Duration,
) -> Result<ThroughputResult> {
    tracing::info!(
        "Starting TCP download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);
    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server: String = server.to_string();
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let addr = format!("{server}:{port}");
            let mut local_samples: Vec<Sample> = Vec::new();

            match TcpStream::connect(&addr).await {
                Ok(mut stream) => {
                    if let Err(e) = stream.set_nodelay(true) {
                        tracing::debug!("TCP set_nodelay failed on download conn {i}: {e}");
                    }
                    if let Err(e) = stream.write_all(b"D").await {
                        tracing::warn!("Failed to send download command on connection {i}: {e}");
                        return local_samples;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;

                    let mut buffer = vec![0u8; read_buffer_size];

                    while start_time.elapsed() < duration {
                        let read_start = Instant::now();
                        let t_start_us = offset_us(start_time, read_start);
                        let is_warmup = start_time.elapsed() < warmup;
                        match stream.read(&mut buffer).await {
                            Ok(0) => {
                                tracing::debug!("Server closed connection {i} (might be normal)");
                                break;
                            }
                            Ok(n) => {
                                let duration_us = read_start.elapsed().as_micros() as u64;
                                let s =
                                    Sample::success(t_start_us, duration_us, n as u64, is_warmup);
                                local_samples.push(s.clone());
                                let _ = tx.send(s);
                            }
                            Err(e) => {
                                let duration_us = read_start.elapsed().as_micros() as u64;
                                let s = Sample::failure(
                                    t_start_us,
                                    duration_us,
                                    ConnectionError::Unknown(e.to_string()),
                                    0,
                                    is_warmup,
                                );
                                local_samples.push(s.clone());
                                let _ = tx.send(s);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("TCP connection error on connection {i}: {e}");
                }
            }

            local_samples
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;

    let streams = collect_streams(results, "download");
    let end_time = Instant::now();

    Ok(ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start_time, end_time, warmup),
        timestamp: chrono::Utc::now(),
        udp_stats: None,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    })
}

async fn run_upload_test(
    server: &str,
    port: u16,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
    warmup: Duration,
) -> Result<ThroughputResult> {
    tracing::info!(
        "Starting TCP upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);
    let start_time = Instant::now();

    let upload_data = {
        let mut data = vec![0u8; payload_size];
        rng().fill_bytes(&mut data);
        data
    };

    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server = server.to_string();
        let data = upload_data.clone();
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let addr = format!("{server}:{port}");
            let mut local_samples: Vec<Sample> = Vec::new();

            // Reconnect-on-error loop. A flaky link can RST a single
            // socket without killing the test; rebuild the connection
            // and keep measuring. Bound the reconnect attempts so we
            // can't busy-loop if the server is gone.
            let mut reconnects_remaining: u32 = 5;
            'outer: while start_time.elapsed() < duration {
                let mut stream = match TcpStream::connect(&addr).await {
                    Ok(s) => s,
                    Err(e) => {
                        let now = Instant::now();
                        let s = Sample::failure(
                            offset_us(start_time, now),
                            0,
                            ConnectionError::ConnectionFailed(format!(
                                "TCP connect (upload) on conn {i}: {e}"
                            )),
                            0,
                            start_time.elapsed() < warmup,
                        );
                        local_samples.push(s.clone());
                        let _ = tx.send(s);
                        if reconnects_remaining == 0 {
                            break 'outer;
                        }
                        reconnects_remaining -= 1;
                        sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                if let Err(e) = stream.set_nodelay(true) {
                    tracing::debug!("TCP set_nodelay failed on upload conn {i}: {e}");
                }
                if let Err(e) = stream.write_all(b"U").await {
                    tracing::warn!("Failed to send upload command on connection {i}: {e}");
                    if reconnects_remaining == 0 {
                        break 'outer;
                    }
                    reconnects_remaining -= 1;
                    continue;
                }

                while start_time.elapsed() < duration {
                    let write_start = Instant::now();
                    let t_start_us = offset_us(start_time, write_start);
                    let is_warmup = start_time.elapsed() < warmup;
                    match stream.write_all(&data).await {
                        Ok(_) => {
                            let duration_us = write_start.elapsed().as_micros() as u64;
                            let s = Sample::success(
                                t_start_us,
                                duration_us,
                                data.len() as u64,
                                is_warmup,
                            );
                            local_samples.push(s.clone());
                            let _ = tx.send(s);
                        }
                        Err(e) => {
                            let duration_us = write_start.elapsed().as_micros() as u64;
                            let s = Sample::failure(
                                t_start_us,
                                duration_us,
                                ConnectionError::TransferFailed(format!(
                                    "TCP write on conn {i}: {e}"
                                )),
                                0,
                                is_warmup,
                            );
                            local_samples.push(s.clone());
                            let _ = tx.send(s);
                            // Connection is dead; break inner loop so
                            // outer reconnects (if budget remains).
                            break;
                        }
                    }
                }
                if reconnects_remaining == 0 {
                    break;
                }
                reconnects_remaining -= 1;
            }

            local_samples
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;

    let streams = collect_streams(results, "upload");
    let end_time = Instant::now();

    Ok(ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start_time, end_time, warmup),
        timestamp: chrono::Utc::now(),
        udp_stats: None,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    })
}
