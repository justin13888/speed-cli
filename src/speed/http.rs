use colored::*;
use eyre::{Context, Result};
use futures::stream::StreamExt;
use rand::{prelude::*, rng};
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tracing::debug;
use url::Url;

use crate::{
    TestType,
    report::{
        HttpTestConfig, HttpTestResult, LatencyMeasurement, LatencyResult, SimpleTestResult,
        TestReport,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpVersion {
    /// HTTP/1.1 without TLS
    HTTP1,
    /// HTTP/2 Cleartext (h2c)
    H2C,
    /// HTTP/2 with TLS
    HTTP2,
    /// HTTP/3 (QUIC)
    HTTP3,
}

use std::fmt;

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpVersion::HTTP1 => write!(f, "HTTP/1.1"),
            HttpVersion::H2C => write!(f, "HTTP/2 Cleartext (h2c)"),
            HttpVersion::HTTP2 => write!(f, "HTTP/2 with TLS"),
            HttpVersion::HTTP3 => write!(f, "HTTP/3 (QUIC)"),
        }
    }
}

pub async fn run_http_test(config: HttpTestConfig) -> Result<TestReport> {
    println!("Starting comprehensive HTTP speed test...");

    let mut result = HttpTestResult {
        test_type: format!("{:?}", config.test_type),
        http_version: format!("{:?}", config.http_version),
        download_mbps: None,
        upload_mbps: None,
        latency_ms: None,
        jitter_ms: None,
        connection_time_ms: 0.0,
        ssl_handshake_ms: None,
        dns_resolution_ms: 0.0,
        parallel_connections: config.parallel_connections,
        bytes_downloaded: 0,
        bytes_uploaded: 0,
        test_duration: Duration::from_secs(config.duration),
        timestamp: chrono::Utc::now(),
        server_url: config.server_url.clone(),
        errors: Vec::new(),
    };

    // Create HTTP client based on version preference
    let client = create_http_client(&config.http_version).await?;

    // Measure DNS resolution time
    let dns_start = Instant::now();
    let url = Url::parse(&config.server_url)?;
    let host = url.host_str().unwrap_or("localhost");
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });

    // Simple DNS resolution timing (basic implementation)
    let _ = tokio::net::lookup_host(format!("{host}:{port}")).await?;
    result.dns_resolution_ms = dns_start.elapsed().as_secs_f64() * 1000.0;

    match config.test_type {
        TestType::LatencyOnly => {
            if let Some(latency) = measure_http_latency(&client, &config.server_url, 10).await? {}
        }
        TestType::Download => {
            let download_result = run_download_test(&client, &config).await?;
            result.download_mbps = Some(download_result.bandwidth_mbps);
            result.bytes_downloaded = download_result.bytes_transferred;
        }
        TestType::Upload => {
            let upload_result = run_upload_test(&client, &config).await?;
            result.upload_mbps = Some(upload_result.bandwidth_mbps);
            result.bytes_uploaded = upload_result.bytes_transferred;
        }
        TestType::Bidirectional => {
            // Run download and upload sequentially
            let download_result = run_download_test(&client, &config).await?;
            result.download_mbps = Some(download_result.bandwidth_mbps);
            result.bytes_downloaded = download_result.bytes_transferred;

            let upload_result = run_upload_test(&client, &config).await?;
            result.upload_mbps = Some(upload_result.bandwidth_mbps);
            result.bytes_uploaded = upload_result.bytes_transferred;
        }
        TestType::Simultaneous => {
            // Run download and upload concurrently
            let (download_result, upload_result) = tokio::join!(
                run_download_test(&client, &config),
                run_upload_test(&client, &config)
            );

            if let Ok(dl) = download_result {
                result.download_mbps = Some(dl.bandwidth_mbps);
                result.bytes_downloaded = dl.bytes_transferred;
            }

            if let Ok(ul) = upload_result {
                result.upload_mbps = Some(ul.bandwidth_mbps);
                result.bytes_uploaded = ul.bytes_transferred;
            }
        }
    }

    Ok((config, result).into())
}

async fn create_http_client(version: &HttpVersion) -> Result<Client> {
    let mut builder = ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .use_rustls_tls();

    match version {
        HttpVersion::HTTP1 => {
            builder = builder.http1_only();
        }
        HttpVersion::HTTP2 => todo!(),
        HttpVersion::H2C => todo!(),
        HttpVersion::HTTP3 => todo!(),
    }

    builder.build().context("Failed to create HTTP client")
}

/// Measure HTTP latency by simply sending HEAD requests to the server
async fn measure_http_latency(
    client: &Client,
    url: &str,
    count: usize,
) -> Result<Option<LatencyResult>> {
    let mut measurements = Vec::new();

    println!("Measuring HTTP latency with {count} requests...");

    let start = Instant::now();
    for i in 0..count {
        match client.head(url).send().await {
            Ok(_response) => {
                let rtt = start.elapsed().as_secs_f64() * 1000.0;
                measurements.push(LatencyMeasurement {
                    rtt_ms: rtt,
                    elapsed_time: start.elapsed(),
                });

                print!(".");
                if (i + 1).is_multiple_of(10) {
                    println!(" {}/{}", i + 1, count);
                }

                // Wait between requests to avoid overwhelming the server
                sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                eprintln!("Latency measurement failed: {e}");
                continue;
            }
        }
    }

    println!();

    if measurements.is_empty() {
        return Ok(None);
    }

    let rtts: Vec<f64> = measurements.iter().map(|m| m.rtt_ms).collect();
    let avg_rtt = rtts.iter().sum::<f64>() / rtts.len() as f64;
    let min_rtt = rtts.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_rtt = rtts.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    // Calculate jitter (standard deviation of RTT)
    let variance = rtts
        .iter()
        .map(|rtt| {
            let diff = rtt - avg_rtt;
            diff * diff
        })
        .sum::<f64>()
        / rtts.len() as f64;
    let jitter = variance.sqrt();

    Ok(Some(LatencyResult {
        avg_rtt,
        min_rtt,
        max_rtt,
        jitter,
        measurements,
    }))
}

async fn run_download_test(client: &Client, config: &HttpTestConfig) -> Result<SimpleTestResult> {
    println!(
        "Starting download test with {} parallel connections...",
        config.parallel_connections
    );

    let start_time = Instant::now();
    let test_duration = Duration::from_secs(config.duration);

    // Determine test file size based on configuration
    let test_sizes = &config.payload_sizes;

    let mut total_downloaded = 0u64;
    let test_start = start_time;

    // Run tests for each configured size (typically just one unless specifically configured)
    for (size_index, test_size) in test_sizes.iter().enumerate() {
        let remaining_duration = test_duration.saturating_sub(test_start.elapsed());
        if remaining_duration.is_zero() {
            break; // No time left for more test sizes
        }

        let size_duration = if test_sizes.len() == 1 {
            remaining_duration
        } else {
            // Divide remaining time among remaining test sizes
            remaining_duration / (test_sizes.len() - size_index) as u32
        };

        let mut tasks = Vec::new();

        for i in 0..config.parallel_connections {
            let client = client.clone();
            let url = format!("{}/download?size={}&id={}", config.server_url, test_size, i);

            let task = tokio::spawn(async move {
                let mut connection_bytes = 0u64;
                let connection_start = Instant::now();

                while connection_start.elapsed() < size_duration {
                    match download_chunk(&client, &url).await {
                        Ok(bytes) => {
                            connection_bytes += bytes;
                        }
                        Err(e) => {
                            eprintln!("Download error on connection {i}: {e}");
                            break;
                        }
                    }
                }

                connection_bytes
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete for this test size
        for task in tasks {
            if let Ok(bytes) = task.await {
                total_downloaded += bytes;
            }
        }
    }

    let actual_duration = start_time.elapsed();

    Ok(SimpleTestResult::new(total_downloaded, actual_duration))
}

async fn run_upload_test(client: &Client, config: &HttpTestConfig) -> Result<SimpleTestResult> {
    println!(
        "Starting upload test with {} parallel connections...",
        config.parallel_connections
    );

    let start_time = Instant::now();
    let test_duration = Duration::from_secs(config.duration);

    // Determine test chunk size based on configuration (same logic as download test)
    let test_sizes = &config.payload_sizes;

    // Use the same size logic as download test for consistent testing
    let mut total_uploaded = 0u64;
    let test_start = start_time;

    // Run tests for each configured size (typically just one unless specifically configured)
    for (size_index, test_size) in test_sizes.iter().enumerate() {
        let remaining_duration = test_duration.saturating_sub(test_start.elapsed());
        if remaining_duration.is_zero() {
            break; // No time left for more test sizes
        }

        let size_duration = if test_sizes.len() == 1 {
            remaining_duration
        } else {
            // Divide remaining time among remaining test sizes
            remaining_duration / (test_sizes.len() - size_index) as u32
        };

        // Generate upload data for this test size
        let mut upload_data = vec![0u8; *test_size];
        rng().fill_bytes(&mut upload_data);

        let mut tasks = Vec::new();

        for i in 0..config.parallel_connections {
            let client = client.clone();
            let url = format!("{}/upload?id={}", config.server_url, i);
            let data = upload_data.clone();

            let task = tokio::spawn(async move {
                let mut connection_bytes = 0u64;
                let connection_start = Instant::now();

                while connection_start.elapsed() < size_duration {
                    match upload_chunk(&client, &url, &data).await {
                        Ok(bytes) => {
                            connection_bytes += bytes;
                        }
                        Err(e) => {
                            eprintln!("Upload error on connection {i}: {e}");
                            break;
                        }
                    }
                }

                connection_bytes
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete for this test size
        for task in tasks {
            if let Ok(bytes) = task.await {
                total_uploaded += bytes;
            }
        }
    }

    let actual_duration = start_time.elapsed();

    Ok(SimpleTestResult::new(total_uploaded, actual_duration))
}

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

/// Determines the optimal download test size based on current connection download throughput.
async fn determine_optimal_download_test_size(
    client: &Client,
    base_url: &str,
    parallel_connections: usize,
) -> Result<usize> {
    // Start with a small test to estimate connection speed
    let small_test_size = 1024 * 1024; // 1MB
    let url = format!("{base_url}/download?size={small_test_size}&test=true");

    let start = Instant::now();
    match download_chunk(client, &url).await {
        Ok(bytes) => {
            let duration = start.elapsed();
            let mbps = (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);

            // Scale test size based on estimated speed and parallel connections
            // Aim for tests that take 5-10 seconds per chunk, divided by number of connections
            let target_duration = 7.0; // seconds
            let base_optimal_size = ((mbps * 1_000_000.0 / 8.0) * target_duration) as usize;

            // Adjust for parallel connections - each connection should handle a reasonable chunk
            let optimal_size = if parallel_connections > 1 {
                (base_optimal_size / parallel_connections).max(512 * 1024) // At least 512KB per connection
            } else {
                base_optimal_size
            };

            // Clamp between 512KB and 100MB
            Ok(optimal_size.clamp(512 * 1024, 100 * 1024 * 1024))
        }
        Err(_) => {
            // Fallback to default size adjusted for parallel connections
            let default_size = 10 * 1024 * 1024; // 10MB
            Ok(if parallel_connections > 1 {
                (default_size / parallel_connections).max(1024 * 1024) // At least 1MB per connection
            } else {
                default_size
            })
        }
    }
}

/// Determines the optimal upload test size based on current connection upload throughput.
async fn determine_optimal_upload_test_size(
    client: &Client,
    base_url: &str,
    parallel_connections: usize,
) -> Result<usize> {
    // Start with a small test to estimate connection speed
    let small_test_size = 1024 * 1024; // 1MB
    let mut test_data = vec![0u8; small_test_size];
    let mut rng = rng();
    rng.fill_bytes(&mut test_data);

    let url = format!("{base_url}/upload?test=true");

    let start = Instant::now();
    match upload_chunk(client, &url, &test_data).await {
        Ok(bytes) => {
            let duration = start.elapsed();
            let mbps = (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);

            // Scale test size based on estimated speed and parallel connections
            // Aim for tests that take 5-10 seconds per chunk, divided by number of connections
            let target_duration = 7.0; // seconds
            let base_optimal_size = ((mbps * 1_000_000.0 / 8.0) * target_duration) as usize;

            // Adjust for parallel connections - each connection should handle a reasonable chunk
            let optimal_size = if parallel_connections > 1 {
                (base_optimal_size / parallel_connections).max(512 * 1024) // At least 512KB per connection
            } else {
                base_optimal_size
            };

            // Clamp between 512KB and 100MB
            Ok(optimal_size.clamp(512 * 1024, 100 * 1024 * 1024))
        }
        Err(_) => {
            // Fallback to default size adjusted for parallel connections
            let default_size = 10 * 1024 * 1024; // 10MB
            Ok(if parallel_connections > 1 {
                (default_size / parallel_connections).max(1024 * 1024) // At least 1MB per connection
            } else {
                default_size
            })
        }
    }
}
