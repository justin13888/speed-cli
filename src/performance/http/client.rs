use chrono::Utc;
use colored::Colorize as _;
use eyre::{Context, Result};
use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use rand::{prelude::*, rng};
use reqwest::{Client, ClientBuilder};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use std::{
    collections::HashMap,
    sync::Once,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    performance::http::HttpVersion,
    report::{
        ConnectionError, HttpTestConfig, HttpTestResult, LatencyMeasurement, LatencyResult,
        TestReport, ThroughputMeasurement, ThroughputResult,
    },
    utils::format::{format_bytes, format_throughput},
};

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    });
}

// TODO: Need to optimize HTTPS (e.g. HTTP/2) tests for throughput

pub async fn run_http_test(config: HttpTestConfig) -> Result<TestReport> {
    println!(
        "{}",
        format!(
            "Starting {} speed test to server {}...",
            config.http_version,
            config.server_url.cyan()
        )
        .green()
        .bold()
    );

    let start_time = Utc::now();

    let mut result = HttpTestResult {
        latency: None,
        download: HashMap::new(),
        upload: HashMap::new(),
        errors: Vec::new(),
    };

    // Create HTTP client based on version preference
    let client = create_http_client(&config.http_version).await?;

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency =
                measure_http_latency(&client, &config.server_url, config.duration).await?;
        }
        TestType::Download => {
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(
                        &client,
                        &config.server_url,
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
                        &client,
                        &config.server_url,
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
                        &client,
                        &config.server_url,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                    )
                    .await?,
                );
                result.upload.insert(
                    *payload_size,
                    run_upload_test(
                        &client,
                        &config.server_url,
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
                        &client,
                        &config.server_url,
                        config.parallel_connections,
                        *payload_size,
                        config.duration,
                    ),
                    run_upload_test(
                        &client,
                        &config.server_url,
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

async fn create_http_client(version: &HttpVersion) -> Result<Client> {
    // Ensure crypto provider is initialized before creating TLS client
    ensure_crypto_provider();

    let mut builder = ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(100)
        .tcp_keepalive(Duration::from_secs(60))
        .tcp_nodelay(true)
        .use_rustls_tls()
        .danger_accept_invalid_certs(true);

    match version {
        HttpVersion::HTTP1 => {
            builder = builder.http1_only();
        }
        HttpVersion::HTTP2 => {
            builder = builder.http2_prior_knowledge();
        }
        HttpVersion::H2C => {
            builder = builder.http2_prior_knowledge();
        }
        HttpVersion::HTTP3 => {
            builder = builder.http3_prior_knowledge();
        }
    }

    builder.build().context("Failed to create HTTP client")
}

/// Measure HTTP latency by simply sending HEAD requests to the server
async fn measure_http_latency(
    client: &Client,
    server_url: &str,
    duration: Duration,
) -> Result<Option<LatencyResult>> {
    let url = format!("{server_url}/latency");
    let mut measurements = Vec::new();

    println!("Measuring HTTP latency for {duration:?}...");

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
                        let request_rate =
                            measurements.len() as f64 / start.elapsed().as_secs_f64();
                        pb.set_message(format!(
                            "Avg latency: {:.2}ms | Req/s: {:.1} | Requests: {}",
                            avg_latency,
                            request_rate,
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
        let request_start = Instant::now();
        match client.head(&url).send().await {
            Ok(_response) => {
                let rtt = request_start.elapsed().as_secs_f64() * 1000.0;
                let measurement = LatencyMeasurement {
                    rtt_ms: Some(rtt),
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());

                // Send to stats collector (non-blocking)
                let _ = tx.send(measurement);
            }
            Err(e) => {
                let measurement = LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());

                // Send to stats collector (non-blocking)
                let _ = tx.send(measurement);

                trace!("HTTP request error while measuring latency: {e}");
            }
        }

        // Wait between requests to avoid overwhelming the server
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
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

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
        let client = client.clone();
        let url = format!("{server_url}/download?size={payload_size}&id={i}");
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let mut local_measurements = Vec::new();
            while start_time.elapsed() < duration {
                let download_start = Instant::now();
                match download_chunk(&client, &url).await {
                    Ok(bytes) => {
                        let measurement =
                            ThroughputMeasurement::new(bytes, download_start.elapsed());
                        local_measurements.push(measurement.clone());
                        let _ = tx.send(measurement);
                    }
                    Err(e) => {
                        let measurements = ThroughputMeasurement::Failure {
                            error: ConnectionError::Unknown(e.to_string()),
                            duration: download_start.elapsed(),
                            retry_count: 0, // No retries in this case
                        };
                        local_measurements.push(measurements.clone());
                        let _ = tx.send(measurements);

                        break;
                    }
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
                    let total_bytes: u64 = measurements
                        .iter()
                        .map(|m| match m {
                            ThroughputMeasurement::Success { bytes, .. } => *bytes,
                            ThroughputMeasurement::Failure { .. } => 0,
                        })
                        .sum();
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
                panic!("Task error: {e}");
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

async fn run_upload_test(
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

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
        let client = client.clone();
        let url = format!("{server_url}/upload?id={i}");
        let data = upload_data.clone();
        let tx = tx.clone();

        let task = tokio::spawn(async move {
            let mut local_measurements = Vec::new();
            while start_time.elapsed() < duration {
                let upload_start = Instant::now();
                match upload_chunk(&client, &url, &data).await {
                    Ok(bytes) => {
                        let measurement = ThroughputMeasurement::new(bytes, upload_start.elapsed());
                        local_measurements.push(measurement.clone());
                        let _ = tx.send(measurement);
                    }
                    Err(e) => {
                        let measurement = ThroughputMeasurement::new_error(
                            ConnectionError::Unknown(e.to_string()),
                            upload_start.elapsed(),
                            0,
                        );
                        local_measurements.push(measurement.clone());
                        let _ = tx.send(measurement);
                    }
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
                    let total_bytes: u64 = measurements
                        .iter()
                        .map(|m| match m {
                            ThroughputMeasurement::Success { bytes, .. } => *bytes,
                            ThroughputMeasurement::Failure { .. } => 0,
                        })
                        .sum();
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
                panic!("Task error: {e}");
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

/// Download a chunk of data from the server
async fn download_chunk(client: &Client, url: &str) -> Result<u64> {
    let response = client.get(url).send().await?;
    let mut total_bytes = 0u64;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total_bytes += chunk.len() as u64;
    }

    Ok(total_bytes)
}

/// Upload a chunk of data to the server
async fn upload_chunk(client: &Client, url: &str, data: &[u8]) -> Result<u64> {
    let response = client
        .post(url)
        .header("Content-Type", "application/octet-stream")
        .body(data.to_vec())
        .send()
        .await?;

    // Ensure the upload was successful
    if response.status().is_success() {
        Ok(data.len() as u64)
    } else {
        eyre::bail!("Upload failed with status: {}", response.status());
    }
}
