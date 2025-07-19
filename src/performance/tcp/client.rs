use chrono::Utc;
use colored::Colorize as _;
use eyre::Result;
use rand::{prelude::*, rng};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, TcpTestConfig, TcpTestResult,
        TestReport, ThroughputMeasurement, ThroughputResult,
    },
    utils::{
        format::format_bytes,
        instrumentation::{
            LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        },
    },
};

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);

    println!(
        "{}",
        format!("Starting TCP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

    let start_time = Utc::now();

    let mut result = TcpTestResult {
        latency: None,
        download: HashMap::new(),
        upload: HashMap::new(),
    };

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
                    ),
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                    )
                );

                result.download.insert(*payload_size, download_result?);
                result.upload.insert(*payload_size, upload_result?);
            }
        }
    }

    Ok((start_time, config, result).into())
}

/// Measure TCP latency by establishing connections and measuring round-trip time
async fn measure_tcp_latency(config: &TcpTestConfig) -> Result<Option<LatencyResult>> {
    let addr = format!("{}:{}", config.server, config.port);
    let duration = config.duration;
    let mut measurements = Vec::new();

    println!("Measuring TCP latency for {duration:?}...");

    // Create progress bar for latency measurement
    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);

    let start = Instant::now();

    // Set up instrumentation
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

    while start.elapsed() < duration {
        let connect_start = Instant::now();
        match TcpStream::connect(&addr).await {
            Ok(mut stream) => {
                let rtt = connect_start.elapsed().as_secs_f64() * 1000.0;
                let measurement = LatencyMeasurement {
                    rtt_ms: Some(rtt),
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());

                // Send to stats collector (non-blocking)
                let _ = tx.send(measurement);

                // Close the connection cleanly
                let _ = stream.shutdown().await;
            }
            Err(e) => {
                let measurement = LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());

                // Send to stats collector (non-blocking)
                let _ = tx.send(measurement);

                trace!("TCP connection error while measuring latency: {e}");
            }
        }

        // Wait between connections to avoid overwhelming the server
        sleep(Duration::from_millis(100)).await;
    }

    // Drop the sender to signal stats collector to finish
    drop(tx);

    // Wait for stats collector to complete and get measurements
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
) -> Result<ThroughputResult> {
    println!(
        "Starting TCP download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);

    let mut measurements = Vec::new();
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
                    // Send download command
                    if let Err(e) = stream.write_all(b"D").await {
                        eprintln!("Failed to send download command on connection {i}: {e}");
                        return local_measurements;
                    }

                    // Give the server a moment to process the command
                    tokio::time::sleep(Duration::from_millis(10)).await;

                    let mut buffer = vec![0u8; payload_size.min(8192)]; // Use smaller buffer sizes to avoid overwhelming

                    while start_time.elapsed() < duration {
                        let read_start = Instant::now();
                        match stream.read(&mut buffer).await {
                            Ok(0) => {
                                // Server closed connection - this might be normal if server hit limits
                                eprintln!("Server closed connection {i} (might be normal)");
                                break;
                            }
                            Ok(n) => {
                                let measurement =
                                    ThroughputMeasurement::new(n as u64, read_start.elapsed());
                                local_measurements.push(measurement.clone());
                                let _ = tx.send(measurement);
                            }
                            Err(e) => {
                                let measurement = ThroughputMeasurement::new_error(
                                    ConnectionError::Unknown(e.to_string()),
                                    read_start.elapsed(),
                                    0,
                                );
                                local_measurements.push(measurement.clone());
                                let _ = tx.send(measurement);
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

    // Drop the sender to signal stats collector to finish
    drop(tx);

    for result in results {
        match result {
            Ok(task_measurements) => {
                measurements.extend(task_measurements);
            }
            Err(e) => {
                panic!("Task error: {e}");
            }
        }
    }

    // Wait for stats collector to complete and get measurements
    measurements = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}

async fn run_upload_test(
    server: &str,
    port: u16,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting TCP upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);

    let mut measurements = Vec::new();
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

            match TcpStream::connect(&addr).await {
                Ok(mut stream) => {
                    // Send upload command
                    if let Err(e) = stream.write_all(b"U").await {
                        eprintln!("Failed to send upload command on connection {i}: {e}");
                        return local_measurements;
                    }

                    while start_time.elapsed() < duration {
                        let write_start = Instant::now();
                        match stream.write_all(&data).await {
                            Ok(_) => {
                                let measurement = ThroughputMeasurement::new(
                                    data.len() as u64,
                                    write_start.elapsed(),
                                );
                                local_measurements.push(measurement.clone());

                                // Send to stats collector (non-blocking)
                                let _ = tx.send(measurement);
                            }
                            Err(e) => {
                                let measurement = ThroughputMeasurement::new_error(
                                    ConnectionError::Unknown(format!(
                                        "TCP write error on connection {i}: {e}"
                                    )),
                                    write_start.elapsed(),
                                    0,
                                );
                                local_measurements.push(measurement.clone());
                                let _ = tx.send(measurement);
                                break;
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

    // Drop the sender to signal stats collector to finish
    drop(tx);

    for result in results {
        match result {
            Ok(task_measurements) => {
                measurements.extend(task_measurements);
            }
            Err(e) => {
                panic!("Task error: {e}");
            }
        }
    }

    // Wait for stats collector to complete and get measurements
    measurements = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}
