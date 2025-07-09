use colored::*;
use eyre::{Context, Result};
use futures::stream::StreamExt;
use rand::{prelude::*, rng};
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::debug;
use url::Url;

use crate::network::*;

#[derive(Debug, Clone)]
pub struct HttpTestConfig {
    pub server_url: String,
    pub duration: u64,
    pub parallel_connections: usize,
    pub test_type: HttpTestType,
    pub http_version: HttpVersion,
    pub test_sizes: Vec<usize>, // Test with different payload sizes
    pub adaptive_sizing: bool,
    pub export_file: Option<String>,
}

#[derive(Debug, Clone)]
pub enum HttpTestType {
    Download,
    Upload,
    Bidirectional,
    LatencyOnly,
    Comprehensive, // All tests
}

#[derive(Debug, Clone)]
pub enum HttpVersion {
    Http11,
    Http2,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestResult {
    pub test_type: String,
    pub http_version: String,
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub latency_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub connection_time_ms: f64,
    pub ssl_handshake_ms: Option<f64>,
    pub dns_resolution_ms: f64,
    pub parallel_connections: usize,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub test_duration: Duration,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub server_url: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LatencyMeasurement {
    pub rtt_ms: f64,
    pub timestamp: Instant,
}

#[derive(Debug)]
pub struct ConnectionMetrics {
    pub dns_time: Duration,
    pub connect_time: Duration,
    pub ssl_time: Option<Duration>,
    pub total_time: Duration,
}

pub async fn run_http_test(config: HttpTestConfig) -> Result<HttpTestResult> {
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
        HttpTestType::LatencyOnly => {
            if let Some(latency) = measure_http_latency(&client, &config.server_url, 10).await? {
                result.latency_ms = Some(latency.avg_rtt);
                result.jitter_ms = Some(latency.jitter);
            }
        }
        HttpTestType::Download => {
            let download_result = run_download_test(&client, &config).await?;
            result.download_mbps = Some(download_result.bandwidth_mbps);
            result.bytes_downloaded = download_result.bytes_transferred;
        }
        HttpTestType::Upload => {
            let upload_result = run_upload_test(&client, &config).await?;
            result.upload_mbps = Some(upload_result.bandwidth_mbps);
            result.bytes_uploaded = upload_result.bytes_transferred;
        }
        HttpTestType::Bidirectional => {
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
        HttpTestType::Comprehensive => {
            // Run all tests in sequence
            println!("{}", "Phase 1: Latency measurement...".yellow());
            if let Ok(Some(latency)) = measure_http_latency(&client, &config.server_url, 10).await {
                result.latency_ms = Some(latency.avg_rtt);
                result.jitter_ms = Some(latency.jitter);
            }

            println!("{}", "Phase 2: Download test...".yellow());
            if let Ok(download_result) = run_download_test(&client, &config).await {
                result.download_mbps = Some(download_result.bandwidth_mbps);
                result.bytes_downloaded = download_result.bytes_transferred;
            }

            println!("{}", "Phase 3: Upload test...".yellow());
            if let Ok(upload_result) = run_upload_test(&client, &config).await {
                result.upload_mbps = Some(upload_result.bandwidth_mbps);
                result.bytes_uploaded = upload_result.bytes_transferred;
            }
        }
    }

    // Export results if requested
    if let Some(export_path) = &config.export_file {
        export_http_results(&result, export_path).await?;
    }

    print_http_results(&result);
    Ok(result)
}

async fn create_http_client(version: &HttpVersion) -> Result<Client> {
    let mut builder = ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .use_rustls_tls();

    match version {
        HttpVersion::Http11 => {
            builder = builder.http1_only();
        }
        HttpVersion::Http2 => {
            // Note: reqwest automatically supports HTTP/2 when available
            // We'll rely on the server negotiating HTTP/2
        }
        HttpVersion::Auto => {
            // Use both HTTP/1.1 and HTTP/2
        }
    }

    builder.build().context("Failed to create HTTP client")
}

#[derive(Debug)]
pub struct LatencyResult {
    pub avg_rtt: f64,
    pub min_rtt: f64,
    pub max_rtt: f64,
    pub jitter: f64,
    pub measurements: Vec<LatencyMeasurement>,
}

async fn measure_http_latency(
    client: &Client,
    url: &str,
    count: usize,
) -> Result<Option<LatencyResult>> {
    let mut measurements = Vec::new();

    println!("Measuring HTTP latency with {count} requests...");

    for i in 0..count {
        let start = Instant::now();

        match client.head(url).send().await {
            Ok(_response) => {
                let rtt = start.elapsed().as_secs_f64() * 1000.0;
                measurements.push(LatencyMeasurement {
                    rtt_ms: rtt,
                    timestamp: start,
                });

                print!(".");
                if (i + 1) % 10 == 0 {
                    println!(" {}/{}", i + 1, count);
                }

                // Wait between requests to avoid overwhelming the server
                sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                eprintln!("Latency measurement failed: {}", e);
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

async fn run_download_test(client: &Client, config: &HttpTestConfig) -> Result<TestResult> {
    println!(
        "Starting download test with {} parallel connections...",
        config.parallel_connections
    );

    let start_time = Instant::now();
    let test_duration = Duration::from_secs(config.duration);

    // Determine test file size based on configuration
    let test_sizes = if config.adaptive_sizing {
        // Start with a small test to estimate speed, then adapt
        let optimal_size = determine_optimal_download_test_size(
            client,
            &config.server_url,
            config.parallel_connections,
        )
        .await?;
        debug!(
            "Adaptive sizing enabled. Optimal test size determined: {} bytes",
            optimal_size
        );
        vec![optimal_size]
    } else if !config.test_sizes.is_empty() {
        config.test_sizes.clone()
    } else {
        vec![10 * 1024 * 1024] // 10MB default
    };

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

    Ok(TestResult::new(total_downloaded, actual_duration))
}

async fn run_upload_test(client: &Client, config: &HttpTestConfig) -> Result<TestResult> {
    println!(
        "Starting upload test with {} parallel connections...",
        config.parallel_connections
    );

    let start_time = Instant::now();
    let test_duration = Duration::from_secs(config.duration);

    // Determine test chunk size based on configuration (same logic as download test)
    let test_sizes = if config.adaptive_sizing {
        // Start with a small test to estimate speed, then adapt
        let optimal_size = determine_optimal_upload_test_size(
            client,
            &config.server_url,
            config.parallel_connections,
        )
        .await?;
        debug!(
            "Adaptive sizing enabled. Optimal upload chunk size determined: {} bytes",
            optimal_size
        );
        vec![optimal_size]
    } else if !config.test_sizes.is_empty() {
        config.test_sizes.clone()
    } else {
        vec![10 * 1024 * 1024] // 10MB default (same as download)
    };

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

    Ok(TestResult::new(total_uploaded, actual_duration))
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

async fn export_http_results(result: &HttpTestResult, path: &str) -> Result<()> {
    if path.ends_with(".json") {
        let json_data = serde_json::to_string_pretty(result)?;
        tokio::fs::write(path, json_data).await?;
    } else if path.ends_with(".csv") {
        // Convert to CSV format
        let csv_content = format!(
            "timestamp,test_type,http_version,download_mbps,upload_mbps,latency_ms,jitter_ms,dns_ms,connection_ms,parallel_connections,bytes_downloaded,bytes_uploaded,duration_s,server_url\n{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            result.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            result.test_type,
            result.http_version,
            result
                .download_mbps
                .map_or("".to_string(), |v| v.to_string()),
            result.upload_mbps.map_or("".to_string(), |v| v.to_string()),
            result.latency_ms.map_or("".to_string(), |v| v.to_string()),
            result.jitter_ms.map_or("".to_string(), |v| v.to_string()),
            result.dns_resolution_ms,
            result.connection_time_ms,
            result.parallel_connections,
            result.bytes_downloaded,
            result.bytes_uploaded,
            result.test_duration.as_secs(),
            result.server_url
        );
        tokio::fs::write(path, csv_content).await?;
    }

    println!("Results exported to {}", path.green());
    Ok(())
}

fn print_http_results(result: &HttpTestResult) {
    println!("\n{}", "HTTP Speed Test Results".green().bold());
    println!("{}", "=".repeat(50).green());

    println!("Test Type: {}", result.test_type.cyan());
    println!("HTTP Version: {}", result.http_version.cyan());
    println!("Server: {}", result.server_url.cyan());
    println!(
        "Parallel Connections: {}",
        result.parallel_connections.to_string().yellow()
    );
    println!("Test Duration: {:.2}s", result.test_duration.as_secs_f64());

    if let Some(download) = result.download_mbps {
        println!(
            "Download Speed: {}",
            format_bandwidth(download).green().bold()
        );
        println!(
            "Data Downloaded: {}",
            format_bytes(result.bytes_downloaded).yellow()
        );
    }

    if let Some(upload) = result.upload_mbps {
        println!("Upload Speed: {}", format_bandwidth(upload).green().bold());
        println!(
            "Data Uploaded: {}",
            format_bytes(result.bytes_uploaded).yellow()
        );
    }

    if let Some(latency) = result.latency_ms {
        println!("Average Latency: {:.2} ms", latency);
        if let Some(jitter) = result.jitter_ms {
            println!("Jitter: {:.2} ms", jitter);
        }
    }

    println!("DNS Resolution: {:.2} ms", result.dns_resolution_ms);

    if !result.errors.is_empty() {
        println!("\n{}:", "Errors".red().bold());
        for error in &result.errors {
            println!("  • {}", error.red());
        }
    }

    println!();
}
