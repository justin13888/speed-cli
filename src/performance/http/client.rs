use chrono::Utc;
use colored::Colorize as _;
use eyre::{Context, Result};
use futures::stream::StreamExt;
use humansize::ToF64;

use rand::{prelude::*, rng};
use reqwest::{Client, ClientBuilder};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use std::{
    sync::Once,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    performance::http::HttpVersion,
    report::{
        ConnectionError, HttpTestConfig, LatencyMeasurement, LatencyResult, NetworkTestResult,
        TestReport, ThroughputMeasurement, ThroughputResult,
    },
    utils::{
        format::format_bytes,
        instrumentation::{
            LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        },
    },
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

    let mut result = NetworkTestResult::new_http();

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
                        config.chunk_size,
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
                        config.chunk_size,
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
                        config.chunk_size,
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
                        config.chunk_size,
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
                        config.chunk_size,
                        config.duration,
                    ),
                    run_upload_test(
                        &client,
                        &config.server_url,
                        config.parallel_connections,
                        *payload_size,
                        config.chunk_size,
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
        HttpVersion::HTTP2 | HttpVersion::H2C => {
            builder = builder
                .http2_prior_knowledge()
                .http2_max_frame_size(Some(65536)) // 64KB (max allowed)
                .http2_adaptive_window(true); // Enable adaptive flow control
        }
        HttpVersion::HTTP3 => {
            builder = builder.http3_prior_knowledge();
            // .http3_congestion_bbr();
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
    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);
    let start = Instant::now();

    // Set up instrumentation
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

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
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    chunk_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting download test with {} payload size and {} parallel connections...",
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
        let client = client.clone();
        let tx = tx.clone();
        let server_url = server_url.to_string();

        let task = tokio::spawn(async move {
            let mut local_measurements = Vec::new();
            while start_time.elapsed() < duration {
                let download_start = Instant::now();
                match download_chunk(&client, &server_url, i, payload_size, chunk_size).await {
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
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    chunk_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Generate random upload data at the size of chunk_size
    let chunk_data = {
        let mut data = vec![0u8; chunk_size];
        rng().fill_bytes(&mut data);
        data
    };
    debug_assert!(chunk_data.len() == chunk_size, "Chunk data size mismatch");

    // Set up instrumentation
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let client = client.clone();
        let tx = tx.clone();
        let server_url = server_url.to_string();
        let chunk_data = chunk_data.clone();

        let task = tokio::spawn(async move {
            let mut local_measurements = Vec::new();
            while start_time.elapsed() < duration {
                let upload_start = Instant::now();
                match upload_chunk(&client, &server_url, payload_size, chunk_data.clone()).await {
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

/// Download a chunk of data from the server
async fn download_chunk(
    client: &Client,
    server_url: &str,
    id: usize,
    payload_size: usize,
    chunk_size: usize,
) -> Result<u64> {
    let response = client
        .get(format!(
            "{server_url}/download?size={payload_size}&chunk_size={chunk_size}&id={id}"
        ))
        .send()
        .await?;
    let mut total_bytes = 0u64;

    let mut stream = response.bytes_stream();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        total_bytes += chunk.len() as u64;
    }

    // Debug assert that total_bytes is within margin of error (10%)
    debug_assert!(
        payload_size.to_f64() * 0.9 <= total_bytes.to_f64()
            && total_bytes.to_f64() <= payload_size.to_f64() * 1.1,
        "Downloaded {total_bytes} bytes, expected within 10% of {payload_size} bytes"
    );

    Ok(total_bytes)
}

async fn upload_chunk(
    client: &Client,
    server_url: &str,
    payload_size: usize,
    chunk_data: Vec<u8>,
) -> Result<u64> {
    let chunk_size = chunk_data.len();
    let total_bytes_to_send = payload_size;
    let mut total_bytes_sent = 0u64;

    // Calculate how many chunks we need to send
    let num_chunks = total_bytes_to_send.div_ceil(chunk_size); // Ceiling division

    for chunk_index in 0..num_chunks {
        let remaining_bytes = total_bytes_to_send - (chunk_index * chunk_size);
        let current_chunk_size = std::cmp::min(chunk_size, remaining_bytes);

        // Use only the needed portion of chunk_data for the last chunk
        let chunk_to_send = if current_chunk_size == chunk_size {
            chunk_data.clone()
        } else {
            chunk_data[..current_chunk_size].to_vec()
        };

        let response = client
            .post(format!("{server_url}/upload"))
            .header("Content-Type", "application/octet-stream")
            .header("X-Chunk-Index", chunk_index.to_string())
            .header("X-Total-Chunks", num_chunks.to_string())
            .body(chunk_to_send)
            .send()
            .await?;

        // Ensure the upload was successful
        if !response.status().is_success() {
            eyre::bail!("Upload failed with status: {}", response.status());
        }

        total_bytes_sent += current_chunk_size as u64;
    }

    Ok(total_bytes_sent)
}
