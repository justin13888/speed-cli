use chrono::Utc;
use colored::Colorize as _;
use eyre::Result;
use indicatif::{ProgressBar, ProgressStyle};
use rand::{prelude::*, rng};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    report::{
        LatencyMeasurement, LatencyResult, TcpTestConfig, TcpTestResult, TestReport,
        ThroughputMeasurement, ThroughputResult,
    },
    utils::format::{format_bytes, format_throughput},
};

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    println!("{}", "Starting TCP speed test...".green().bold());

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
                    run_download_test(&config, *payload_size).await?,
                );
            }
        }
        TestType::Upload => {
            for payload_size in &config.payload_sizes {
                result.upload.insert(
                    *payload_size,
                    run_upload_test(&config, *payload_size).await?,
                );
            }
        }
        TestType::Bidirectional => {
            // Run download and upload sequentially
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(&config, *payload_size).await?,
                );
                result.upload.insert(
                    *payload_size,
                    run_upload_test(&config, *payload_size).await?,
                );
            }
        }
        TestType::Simultaneous => {
            // Run download and upload concurrently
            for payload_size in &config.payload_sizes {
                let (download_result, upload_result) = tokio::join!(
                    run_download_test(&config, *payload_size),
                    run_upload_test(&config, *payload_size)
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
    let progress_bar = ProgressBar::new(duration.as_secs());
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.yellow/blue} {pos}s/{len}s {msg}")
            .unwrap()
            .progress_chars("##-"),
    );
    progress_bar.set_message("Measuring latency...");

    let start = Instant::now();

    // Use mpsc channel instead of Arc<Mutex<Vec<T>>>
    let (tx, mut rx) = mpsc::unbounded_channel::<LatencyMeasurement>();

    // Spawn a task to collect measurements for statistics
    let stats_measurements =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::<LatencyMeasurement>::new()));
    let stats_measurements_clone = stats_measurements.clone();

    let stats_collector = tokio::spawn(async move {
        while let Some(measurement) = rx.recv().await {
            if let Ok(mut measurements) = stats_measurements_clone.lock() {
                measurements.push(measurement);
            }
        }
    });

    let progress_task = {
        let pb = progress_bar.clone();
        let measurements_for_stats = stats_measurements.clone();
        tokio::spawn(async move {
            while start.elapsed() < duration {
                let elapsed = start.elapsed().as_secs();
                pb.set_position(elapsed);

                // Calculate and display running average latency
                if let Ok(measurements) = measurements_for_stats.lock()
                    && !measurements.is_empty()
                {
                    let valid_measurements: Vec<f64> =
                        measurements.iter().filter_map(|m| m.rtt_ms).collect();

                    if !valid_measurements.is_empty() {
                        let avg_latency = valid_measurements.iter().sum::<f64>()
                            / valid_measurements.len() as f64;
                        let connection_rate =
                            measurements.len() as f64 / start.elapsed().as_secs_f64();
                        pb.set_message(format!(
                            "Avg latency: {:.2}ms | Conn/s: {:.1} | Connections: {}",
                            avg_latency,
                            connection_rate,
                            measurements.len()
                        ));
                    }
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            pb.set_position(duration.as_secs());
        })
    };

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

    // Wait for stats collector and progress task to complete
    let _ = tokio::join!(stats_collector, progress_task);
    progress_bar.finish_with_message("Latency measurement complete");

    if measurements.is_empty() {
        return Ok(None);
    }

    Ok(Some(LatencyResult {
        measurements,
        timestamp: chrono::Utc::now(),
    }))
}

async fn run_download_test(
    config: &TcpTestConfig,
    payload_size: usize,
) -> Result<ThroughputResult> {
    println!(
        "Starting TCP download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        config.parallel_connections.to_string().yellow()
    );

    let duration = config.duration;
    let parallel_connections = config.parallel_connections;
    let server = config.server.clone();
    let port = config.port;

    // Create progress bar
    let progress_bar = ProgressBar::new(duration.as_secs());
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}s/{len}s {msg}")
            .unwrap()
            .progress_chars("##-"),
    );
    progress_bar.set_message("Downloading...");

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Use mpsc channel instead of Arc<Mutex<Vec<T>>>
    let (tx, mut rx) = mpsc::unbounded_channel::<ThroughputMeasurement>();

    // Spawn a task to collect measurements for statistics
    let stats_measurements =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::<ThroughputMeasurement>::new()));
    let stats_measurements_clone = stats_measurements.clone();

    let stats_collector = tokio::spawn(async move {
        while let Some(measurement) = rx.recv().await {
            if let Ok(mut measurements) = stats_measurements_clone.lock() {
                measurements.push(measurement);
            }
        }
    });

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server = server.clone();
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
                                let measurement = ThroughputMeasurement {
                                    bytes: n as u64,
                                    duration: read_start.elapsed(),
                                };
                                local_measurements.push(measurement.clone());

                                // Send to stats collector (non-blocking)
                                let _ = tx.send(measurement);
                            }
                            Err(e) => {
                                eprintln!("TCP read error on connection {i}: {e}");
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

    // Update progress bar in a separate task
    let progress_task = {
        let pb = progress_bar.clone();
        let measurements_for_stats = stats_measurements.clone();
        tokio::spawn(async move {
            while start_time.elapsed() < duration {
                let elapsed = start_time.elapsed().as_secs();
                pb.set_position(elapsed);

                // Calculate and display running average throughput
                if let Ok(measurements) = measurements_for_stats.lock()
                    && !measurements.is_empty()
                {
                    let total_bytes: u64 = measurements.iter().map(|m| m.bytes).sum();
                    let elapsed_secs = start_time.elapsed().as_secs_f64();
                    let throughput_mbps = (total_bytes as f64 * 8.0) / (elapsed_secs * 1_000_000.0);
                    let throughput_bytes_per_sec = total_bytes as f64 / elapsed_secs;

                    pb.set_message(format!(
                        "Avg: {} | {} | Chunks: {}",
                        format_throughput(throughput_mbps),
                        format_bytes(throughput_bytes_per_sec as usize),
                        measurements.len()
                    ));
                }

                // Update every 100ms
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            pb.set_position(duration.as_secs());
        })
    };

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
                eprintln!("Task error: {e}");
            }
        }
    }

    // Wait for stats collector and progress task to complete
    let _ = tokio::join!(stats_collector, progress_task);
    progress_bar.finish_with_message("Download complete");

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}

async fn run_upload_test(config: &TcpTestConfig, payload_size: usize) -> Result<ThroughputResult> {
    println!(
        "Starting TCP upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        config.parallel_connections.to_string().yellow()
    );

    let duration = config.duration;
    let parallel_connections = config.parallel_connections;
    let server = config.server.clone();
    let port = config.port;

    // Create progress bar
    let progress_bar = ProgressBar::new(duration.as_secs());
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/blue} {pos}s/{len}s {msg}")
            .unwrap()
            .progress_chars("##-"),
    );
    progress_bar.set_message("Uploading...");

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Generate upload data
    let upload_data = {
        let mut data = vec![0u8; payload_size];
        rng().fill_bytes(&mut data);
        data
    };

    // Use mpsc channel instead of Arc<Mutex<Vec<T>>>
    let (tx, mut rx) = mpsc::unbounded_channel::<ThroughputMeasurement>();

    // Spawn a task to collect measurements for statistics
    let stats_measurements =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::<ThroughputMeasurement>::new()));
    let stats_measurements_clone = stats_measurements.clone();

    let stats_collector = tokio::spawn(async move {
        while let Some(measurement) = rx.recv().await {
            if let Ok(mut measurements) = stats_measurements_clone.lock() {
                measurements.push(measurement);
            }
        }
    });

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let server = server.clone();
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
                                let measurement = ThroughputMeasurement {
                                    bytes: data.len() as u64,
                                    duration: write_start.elapsed(),
                                };
                                local_measurements.push(measurement.clone());

                                // Send to stats collector (non-blocking)
                                let _ = tx.send(measurement);
                            }
                            Err(e) => {
                                eprintln!("TCP write error on connection {i}: {e}");
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

    // Update progress bar in a separate task
    let progress_task = {
        let pb = progress_bar.clone();
        let measurements_for_stats = stats_measurements.clone();
        tokio::spawn(async move {
            while start_time.elapsed() < duration {
                let elapsed = start_time.elapsed().as_secs();
                pb.set_position(elapsed);

                // Calculate and display running average throughput
                if let Ok(measurements) = measurements_for_stats.lock()
                    && !measurements.is_empty()
                {
                    let total_bytes: u64 = measurements.iter().map(|m| m.bytes).sum();
                    let elapsed_secs = start_time.elapsed().as_secs_f64();
                    let throughput_mbps = (total_bytes as f64 * 8.0) / (elapsed_secs * 1_000_000.0);
                    let throughput_bytes_per_sec = total_bytes as f64 / elapsed_secs;

                    pb.set_message(format!(
                        "Avg: {} | {} | Chunks: {}",
                        format_throughput(throughput_mbps),
                        format_bytes(throughput_bytes_per_sec as usize),
                        measurements.len()
                    ));
                }

                // Update every 100ms
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            pb.set_position(duration.as_secs());
        })
    };

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
                eprintln!("Task error: {e}");
            }
        }
    }

    // Wait for stats collector and progress task to complete
    let _ = tokio::join!(stats_collector, progress_task);
    progress_bar.finish_with_message("Upload complete");

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}
