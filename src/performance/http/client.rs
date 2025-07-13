use chrono::Utc;
use colored::Colorize as _;
use eyre::{Context, Result};
use futures::stream::StreamExt;
use rand::{prelude::*, rng};
use reqwest::{Client, ClientBuilder};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use std::{
    collections::HashMap,
    sync::Once,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tracing::trace;

use crate::{
    TestType,
    performance::http::HttpVersion,
    report::{
        HttpTestConfig, HttpTestResult, LatencyMeasurement, LatencyResult, TestReport,
        ThroughputMeasurement, ThroughputResult,
    },
};

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    });
}

pub async fn run_http_test(config: HttpTestConfig) -> Result<TestReport> {
    println!("{}", "Starting HTTP speed test...".green().bold());

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
            result.latency = measure_http_latency(&client, &config.server_url, 10).await?;
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
                    rtt_ms: Some(rtt),
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
                measurements.push(LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                });
                trace!("HTTP request error while measuring latency: {e}");
            }
        }
    }

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
        "Starting download test with {payload_size} payload size and {} parallel connections...",
        parallel_connections
    );

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let client = client.clone();
        let url = format!("{}/download?size={}&id={}", server_url, payload_size, i);

        let task = tokio::spawn(async move {
            let mut measurements = Vec::new();
            while start_time.elapsed() < duration {
                let download_start = Instant::now();
                match download_chunk(&client, &url).await {
                    Ok(bytes) => {
                        measurements.push(ThroughputMeasurement {
                            bytes,
                            duration: download_start.elapsed(),
                        });
                    }
                    Err(e) => {
                        eprintln!("Download error on connection {i}: {e}");
                        break;
                    }
                }
            }

            measurements
        });

        tasks.push(task);
    }

    for task in tasks {
        match task.await {
            Ok(task_measurements) => {
                measurements.extend(task_measurements);
            }
            Err(e) => {
                eprintln!("Task error: {e}");
            }
        }
    }

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
        "Starting upload test with {payload_size} payload size and {parallel_connections} parallel connections...",
    );

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Generate upload data
    let mut upload_data = vec![0u8; payload_size];
    rng().fill_bytes(&mut upload_data);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let client = client.clone();
        let url = format!("{}/upload?id={}", server_url, i);
        let data = upload_data.clone();

        let task = tokio::spawn(async move {
            let mut measurements = Vec::new();
            while start_time.elapsed() < duration {
                let upload_start = Instant::now();
                match upload_chunk(&client, &url, &data).await {
                    Ok(bytes) => {
                        measurements.push(ThroughputMeasurement {
                            bytes,
                            duration: upload_start.elapsed(),
                        });
                    }
                    Err(e) => {
                        eprintln!("Upload error on connection {i}: {e}");
                        break;
                    }
                }
            }

            measurements
        });

        tasks.push(task);
    }

    for task in tasks {
        match task.await {
            Ok(task_measurements) => {
                measurements.extend(task_measurements);
            }
            Err(e) => {
                eprintln!("Task error: {e}");
            }
        }
    }

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
