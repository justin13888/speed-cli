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
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, StreamMeasurements,
        TcpTestConfig, TestReport, ThroughputMeasurement, ThroughputResult,
    },
    utils::{
        format::format_bytes,
        instrumentation::{
            LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        },
    },
};

/// Effective measurement duration: total elapsed minus the warmup window,
/// clamped so we never report a zero or negative duration that would blow up
/// throughput calculations.
fn measurement_duration(start: Instant, end: Instant, warmup: Duration) -> Duration {
    end.duration_since(start)
        .saturating_sub(warmup)
        .max(Duration::from_millis(1))
}

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
    eprintln!(
        "Starting TCP full-duplex test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let dl_pb = create_progress_bar(ProgressBarType::Download, duration);
    let ul_pb = create_progress_bar(ProgressBarType::Upload, duration);
    let start_time = Instant::now();

    let (dl_collector, dl_tx) =
        ThroughputStatsCollector::new(dl_pb.clone(), start_time, duration);
    let (ul_collector, ul_tx) =
        ThroughputStatsCollector::new(ul_pb.clone(), start_time, duration);

    let mut tasks: Vec<
        tokio::task::JoinHandle<(Vec<ThroughputMeasurement>, Vec<ThroughputMeasurement>)>,
    > = Vec::with_capacity(parallel_connections);

    for i in 0..parallel_connections {
        let server = server.to_string();
        let dl_tx = dl_tx.clone();
        let ul_tx = ul_tx.clone();

        let task = tokio::spawn(async move {
            let mut dl_local = Vec::new();
            let mut ul_local = Vec::new();
            let addr = format!("{server}:{port}");

            let stream = match TcpStream::connect(&addr).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("TCP full-duplex connect error on conn {i}: {e}");
                    return (dl_local, ul_local);
                }
            };
            if let Err(e) = stream.set_nodelay(true) {
                tracing::debug!("TCP set_nodelay failed on full-duplex conn {i}: {e}");
            }
            let (mut read_half, mut write_half) = stream.into_split();

            // Send the F command to put the server into full-duplex mode.
            if let Err(e) = write_half.write_all(b"F").await {
                eprintln!("Failed to send F command on conn {i}: {e}");
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
                    let in_warmup = start_time.elapsed() < warmup;
                    match read_half.read(&mut read_buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let m = ThroughputMeasurement::new(n as u64, read_start.elapsed());
                            if !in_warmup {
                                dl_local.push(m.clone());
                                let _ = dl_tx.send(m);
                            }
                        }
                        Err(e) => {
                            let m = ThroughputMeasurement::new_error(
                                ConnectionError::Unknown(e.to_string()),
                                read_start.elapsed(),
                                0,
                            );
                            if !in_warmup {
                                dl_local.push(m.clone());
                                let _ = dl_tx.send(m);
                            }
                            break;
                        }
                    }
                }
            };

            let write_fut = async {
                while start_time.elapsed() < duration {
                    let write_start = Instant::now();
                    let in_warmup = start_time.elapsed() < warmup;
                    match write_half.write_all(&upload_data).await {
                        Ok(()) => {
                            let m = ThroughputMeasurement::new(
                                upload_data.len() as u64,
                                write_start.elapsed(),
                            );
                            if !in_warmup {
                                ul_local.push(m.clone());
                                let _ = ul_tx.send(m);
                            }
                        }
                        Err(e) => {
                            let m = ThroughputMeasurement::new_error(
                                ConnectionError::Unknown(format!(
                                    "TCP full-duplex write error on conn {i}: {e}"
                                )),
                                write_start.elapsed(),
                                0,
                            );
                            if !in_warmup {
                                ul_local.push(m.clone());
                                let _ = ul_tx.send(m);
                            }
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

    let mut dl_streams = Vec::with_capacity(results.len());
    let mut ul_streams = Vec::with_capacity(results.len());
    let mut dl_flat = Vec::new();
    let mut ul_flat = Vec::new();
    for (idx, joined) in results.into_iter().enumerate() {
        match joined {
            Ok((dl, ul)) => {
                dl_flat.extend(dl.iter().cloned());
                ul_flat.extend(ul.iter().cloned());
                dl_streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: dl,
                });
                ul_streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: ul,
                });
            }
            Err(e) => {
                tracing::error!("full-duplex task {idx} panicked or was cancelled: {e}");
                dl_streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: Vec::new(),
                });
                ul_streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: Vec::new(),
                });
            }
        }
    }

    let end_time = Instant::now();
    let duration_eff = measurement_duration(start_time, end_time, warmup);
    let timestamp = chrono::Utc::now();

    Ok((
        ThroughputResult {
            measurements: dl_flat,
            streams: dl_streams,
            total_duration: duration_eff,
            timestamp,
            udp_stats: None,
        },
        ThroughputResult {
            measurements: ul_flat,
            streams: ul_streams,
            total_duration: duration_eff,
            timestamp,
            udp_stats: None,
        },
    ))
}

/// Take the join_all results from a multi-stream throughput test and
/// produce (per-stream measurements, flattened aggregate). Logs any task
/// that panicked rather than failing the whole test.
fn collect_streams(
    results: Vec<Result<Vec<ThroughputMeasurement>, tokio::task::JoinError>>,
    direction: &'static str,
) -> (Vec<StreamMeasurements>, Vec<ThroughputMeasurement>) {
    let mut streams = Vec::with_capacity(results.len());
    let mut flat = Vec::new();
    for (idx, result) in results.into_iter().enumerate() {
        match result {
            Ok(task_measurements) => {
                flat.extend(task_measurements.iter().cloned());
                streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: task_measurements,
                });
            }
            Err(e) => {
                tracing::error!("{direction} task {idx} panicked or was cancelled: {e}");
                // Still emit an empty stream entry so the per-stream rows
                // are aligned with the connection indices.
                streams.push(StreamMeasurements {
                    stream_id: idx,
                    measurements: Vec::new(),
                });
            }
        }
    }
    (streams, flat)
}

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);

    eprintln!(
        "{}",
        format!("Starting TCP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

    // Pre-flight: confirm we can reach the server before running the full
    // test. Saves the user a 30-second timeout when the server is down.
    match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&server_addr)).await {
        Ok(Ok(stream)) => drop(stream),
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
    }

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

    Ok((start_time, config, result).into())
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

    eprintln!("Measuring TCP in-stream RTT for {duration:?}...");

    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);
    let start = Instant::now();
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

    let mut stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            return Err(eyre::eyre!("TCP latency: connect to {} failed: {}", addr, e));
        }
    };
    if let Err(e) = stream.set_nodelay(true) {
        tracing::debug!("TCP set_nodelay failed on latency stream: {e}");
    }
    if let Err(e) = stream.write_all(b"P").await {
        return Err(eyre::eyre!("TCP latency: failed to send 'P' command: {}", e));
    }
    // Drain the (potential) idle period so the server is in select.
    sleep(Duration::from_millis(10)).await;

    let mut send_buf = [0u8; 8];
    let mut recv_buf = [0u8; 8];
    // Probe ~every 10ms (well below the floor where added latency
    // would matter). Configurable knob is a good follow-up.
    let probe_interval = Duration::from_millis(10);

    while start.elapsed() < duration {
        let in_warmup = start.elapsed() < warmup;
        let probe_start = Instant::now();
        // Write a fresh nonce so we can detect if the echo is offset
        // (would only happen on a corrupted stream; we just discard).
        let nonce: u64 = probe_start.elapsed().as_micros() as u64;
        send_buf.copy_from_slice(&nonce.to_le_bytes());
        let measurement = match stream.write_all(&send_buf).await {
            Ok(()) => {
                match tokio::time::timeout(
                    Duration::from_secs(2),
                    stream.read_exact(&mut recv_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        let echoed = u64::from_le_bytes(recv_buf);
                        if echoed != nonce {
                            tracing::debug!("TCP latency: nonce mismatch, discarding sample");
                            LatencyMeasurement {
                                rtt_ms: None,
                                elapsed_time: start.elapsed(),
                            }
                        } else {
                            LatencyMeasurement {
                                rtt_ms: Some(probe_start.elapsed().as_secs_f64() * 1000.0),
                                elapsed_time: start.elapsed(),
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        trace!("TCP latency read error: {e}");
                        LatencyMeasurement {
                            rtt_ms: None,
                            elapsed_time: start.elapsed(),
                        }
                    }
                    Err(_) => {
                        tracing::debug!("TCP latency: read timeout, dropped sample");
                        LatencyMeasurement {
                            rtt_ms: None,
                            elapsed_time: start.elapsed(),
                        }
                    }
                }
            }
            Err(e) => {
                trace!("TCP latency write error: {e}");
                LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                }
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
    eprintln!(
        "Starting TCP download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);

    let start_time = Instant::now();

    // Set up instrumentation
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server: String = server.to_string();
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let addr = format!("{server}:{port}");
            let mut local_measurements = Vec::new();

            match TcpStream::connect(&addr).await {
                Ok(mut stream) => {
                    if let Err(e) = stream.set_nodelay(true) {
                        tracing::debug!("TCP set_nodelay failed on download conn {i}: {e}");
                    }
                    // Send download command
                    if let Err(e) = stream.write_all(b"D").await {
                        eprintln!("Failed to send download command on connection {i}: {e}");
                        return local_measurements;
                    }

                    // Give the server a moment to process the command
                    tokio::time::sleep(Duration::from_millis(10)).await;

                    let mut buffer = vec![0u8; read_buffer_size];

                    while start_time.elapsed() < duration {
                        let read_start = Instant::now();
                        let in_warmup = start_time.elapsed() < warmup;
                        match stream.read(&mut buffer).await {
                            Ok(0) => {
                                // Server closed connection - this might be normal if server hit limits
                                eprintln!("Server closed connection {i} (might be normal)");
                                break;
                            }
                            Ok(n) => {
                                let measurement =
                                    ThroughputMeasurement::new(n as u64, read_start.elapsed());
                                if !in_warmup {
                                    local_measurements.push(measurement.clone());
                                    let _ = tx.send(measurement);
                                }
                            }
                            Err(e) => {
                                let measurement = ThroughputMeasurement::new_error(
                                    ConnectionError::Unknown(e.to_string()),
                                    read_start.elapsed(),
                                    0,
                                );
                                if !in_warmup {
                                    local_measurements.push(measurement.clone());
                                    let _ = tx.send(measurement);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("TCP connection error on connection {i}: {e}");
                }
            }

            local_measurements
        });

        tasks.push(task);
    }

    // Wait for all tasks to complete concurrently
    let results = futures::future::join_all(tasks).await;

    // Drop the sender so the live-progress collector can clean up.
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;

    // Build per-stream + flattened aggregate from the per-task results.
    let (streams, measurements) = collect_streams(results, "download");

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        streams,
        total_duration: measurement_duration(start_time, end_time, warmup),
        timestamp: chrono::Utc::now(),
        udp_stats: None,
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
    eprintln!(
        "Starting TCP upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);

    let start_time = Instant::now();

    // Generate upload data
    let upload_data = {
        let mut data = vec![0u8; payload_size];
        rng().fill_bytes(&mut data);
        data
    };

    // Set up instrumentation
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server = server.to_string();
        let data = upload_data.clone();
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let addr = format!("{server}:{port}");
            let mut local_measurements = Vec::new();

            // Reconnect-on-error loop. A flaky link can RST a single
            // socket without killing the test; rebuild the connection
            // and keep measuring. Bound the reconnect attempts so we
            // can't busy-loop if the server is gone.
            let mut reconnects_remaining: u32 = 5;
            'outer: while start_time.elapsed() < duration {
                let mut stream = match TcpStream::connect(&addr).await {
                    Ok(s) => s,
                    Err(e) => {
                        let measurement = ThroughputMeasurement::new_error(
                            ConnectionError::ConnectionFailed(format!(
                                "TCP connect (upload) on conn {i}: {e}"
                            )),
                            Duration::from_millis(0),
                            0,
                        );
                        local_measurements.push(measurement.clone());
                        let _ = tx.send(measurement);
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
                    eprintln!("Failed to send upload command on connection {i}: {e}");
                    if reconnects_remaining == 0 {
                        break 'outer;
                    }
                    reconnects_remaining -= 1;
                    continue;
                }

                while start_time.elapsed() < duration {
                    let write_start = Instant::now();
                    let in_warmup = start_time.elapsed() < warmup;
                    match stream.write_all(&data).await {
                        Ok(_) => {
                            let measurement = ThroughputMeasurement::new(
                                data.len() as u64,
                                write_start.elapsed(),
                            );
                            if !in_warmup {
                                local_measurements.push(measurement.clone());
                                let _ = tx.send(measurement);
                            }
                        }
                        Err(e) => {
                            let measurement = ThroughputMeasurement::new_error(
                                ConnectionError::TransferFailed(format!(
                                    "TCP write on conn {i}: {e}"
                                )),
                                write_start.elapsed(),
                                0,
                            );
                            if !in_warmup {
                                local_measurements.push(measurement.clone());
                                let _ = tx.send(measurement);
                            }
                            // Connection is dead; break inner loop so
                            // outer reconnects (if budget remains).
                            break;
                        }
                    }
                }
                // Inner loop exited - either duration elapsed or stream
                // died. If duration elapsed, outer loop will exit too.
                if reconnects_remaining == 0 {
                    break;
                }
                reconnects_remaining -= 1;
            }

            local_measurements
        });

        tasks.push(task);
    }

    // Wait for all tasks to complete concurrently
    let results = futures::future::join_all(tasks).await;

    // Drop the sender so the live-progress collector can clean up.
    drop(tx);
    let _ = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;

    let (streams, measurements) = collect_streams(results, "upload");

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        streams,
        total_duration: measurement_duration(start_time, end_time, warmup),
        timestamp: chrono::Utc::now(),
        udp_stats: None,
    })
}
