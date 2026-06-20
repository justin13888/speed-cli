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
    performance::engine::{
        LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        measurement_duration_us, offset_us,
    },
    performance::http::HttpVersion,
    performance::http::server::decode_base64_urlsafe,
    report::{
        ConnectionError, HttpTestConfig, LatencyMeasurement, LatencyResult, NetworkTestResult,
        PeerIdentity, Sample, StreamSamples, TestReport, ThroughputResult,
    },
    utils::format::format_bytes,
};

const SERVER_ID_HEADER: &str = "x-speed-cli-server-id";

/// reqwest routes a request to its HTTP/3 transport only when the
/// request's version is explicitly `HTTP_3` — `http3_prior_knowledge()`
/// alone is not enough. Apply that version for the HTTP/3 mode.
fn apply_version(rb: reqwest::RequestBuilder, version: HttpVersion) -> reqwest::RequestBuilder {
    if matches!(version, HttpVersion::HTTP3) {
        rb.version(reqwest::Version::HTTP_3)
    } else {
        rb
    }
}

fn parse_server_identity(resp: &reqwest::Response) -> Option<PeerIdentity> {
    let value = resp.headers().get(SERVER_ID_HEADER)?.to_str().ok()?;
    let bytes = decode_base64_urlsafe(value)?;
    ciborium::from_reader::<PeerIdentity, _>(bytes.as_slice()).ok()
}

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
                tracing::error!("HTTP {direction} task {idx} panicked or was cancelled: {e}");
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

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());
    });
}

// TODO: Need to optimize HTTPS (e.g. HTTP/2) tests for throughput

pub async fn run_http_test(config: HttpTestConfig) -> Result<TestReport> {
    eprintln!(
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

    let mut result = NetworkTestResult::new_http().with_accounting(config.accounting);

    let client = create_http_client(&config.http_version).await?;

    let info_url = format!("{}/info", config.server_url);
    let server_identity: Option<PeerIdentity>;
    let preflight_remote = match tokio::time::timeout(
        Duration::from_secs(5),
        apply_version(client.get(&info_url), config.http_version).send(),
    )
    .await
    {
        Ok(Ok(resp)) if resp.status().is_success() => {
            tracing::debug!("Server pre-flight check passed: {}", info_url);
            server_identity = parse_server_identity(&resp);
            resp.remote_addr()
        }
        Ok(Ok(resp)) => {
            return Err(eyre::eyre!(
                "Server pre-flight check returned status {} for {}",
                resp.status(),
                info_url
            ));
        }
        Ok(Err(e)) => {
            return Err(eyre::eyre!(
                "Server pre-flight check failed for {}: {}",
                info_url,
                e
            ));
        }
        Err(_) => {
            return Err(eyre::eyre!(
                "Server pre-flight check timed out after 5s ({})",
                info_url
            ));
        }
    };

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = measure_http_latency(
                &client,
                &config.server_url,
                config.duration,
                config.warmup,
                config.http_version,
            )
            .await?;
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
                        config.warmup,
                        config.http_version,
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
                        config.warmup,
                        config.http_version,
                    )
                    .await?,
                );
            }
        }
        TestType::Bidirectional => {
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
                        config.warmup,
                        config.http_version,
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
                        config.warmup,
                        config.http_version,
                    )
                    .await?,
                );
            }
        }
        TestType::Simultaneous => {
            for payload_size in &config.payload_sizes {
                let (download_result, upload_result) = tokio::join!(
                    run_download_test(
                        &client,
                        &config.server_url,
                        config.parallel_connections,
                        *payload_size,
                        config.chunk_size,
                        config.duration,
                        config.warmup,
                        config.http_version,
                    ),
                    run_upload_test(
                        &client,
                        &config.server_url,
                        config.parallel_connections,
                        *payload_size,
                        config.chunk_size,
                        config.duration,
                        config.warmup,
                        config.http_version,
                    )
                );

                result.download.insert(*payload_size, download_result?);
                result.upload.insert(*payload_size, upload_result?);
            }
        }
        TestType::FullDuplex => {
            return Err(eyre::eyre!(
                "FullDuplex test type is TCP-only; HTTP is request/response. \
                 Use --type=simultaneous for parallel up/down on HTTP."
            ));
        }
    }

    let mut report: TestReport = (start_time, config, result).into();
    // Reqwest doesn't expose the underlying socket's local_addr; we
    // record the server-side address only.
    report.peers.client.remote_addr = preflight_remote;
    report.peers.server.identity = server_identity;
    Ok(report)
}

async fn create_http_client(version: &HttpVersion) -> Result<Client> {
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
                .http2_max_frame_size(Some(65536))
                .http2_adaptive_window(true);
        }
        HttpVersion::HTTP3 => {
            builder = builder.http3_prior_knowledge();
        }
    }

    builder.build().context("Failed to create HTTP client")
}

async fn measure_http_latency(
    client: &Client,
    server_url: &str,
    duration: Duration,
    warmup: Duration,
    version: HttpVersion,
) -> Result<Option<LatencyResult>> {
    let url = format!("{server_url}/latency");
    let mut measurements = Vec::new();

    eprintln!("Measuring HTTP latency for {duration:?}...");

    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);
    let start = Instant::now();
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

    while start.elapsed() < duration {
        let request_start = Instant::now();
        let t_start_us = offset_us(start, request_start);
        let in_warmup = start.elapsed() < warmup;
        match apply_version(client.head(&url), version).send().await {
            Ok(_response) => {
                let rtt_us = request_start.elapsed().as_micros() as u64;
                let measurement = LatencyMeasurement::success(t_start_us, rtt_us);
                if !in_warmup {
                    measurements.push(measurement.clone());
                    let _ = tx.send(measurement);
                }
            }
            Err(e) => {
                let measurement = LatencyMeasurement::dropped(t_start_us);
                if !in_warmup {
                    measurements.push(measurement.clone());
                    let _ = tx.send(measurement);
                }
                trace!("HTTP request error while measuring latency: {e}");
            }
        }

        sleep(Duration::from_millis(100)).await;
    }

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

#[allow(clippy::too_many_arguments)]
async fn run_download_test(
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    chunk_size: usize,
    duration: Duration,
    warmup: Duration,
    version: HttpVersion,
) -> Result<ThroughputResult> {
    eprintln!(
        "Starting download test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);
    let start_time = Instant::now();
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for i in 0..parallel_connections {
        let client = client.clone();
        let tx = tx.clone();
        let server_url = server_url.to_string();

        let task = tokio::spawn(async move {
            let mut local_samples: Vec<Sample> = Vec::new();
            while start_time.elapsed() < duration {
                let download_start = Instant::now();
                let t_start_us = offset_us(start_time, download_start);
                let is_warmup = start_time.elapsed() < warmup;
                match download_chunk(&client, &server_url, i, payload_size, chunk_size, version)
                    .await
                {
                    Ok(bytes) => {
                        let duration_us = download_start.elapsed().as_micros() as u64;
                        let s = Sample::success(t_start_us, duration_us, bytes, is_warmup);
                        local_samples.push(s.clone());
                        let _ = tx.send(s);
                    }
                    Err(e) => {
                        let duration_us = download_start.elapsed().as_micros() as u64;
                        let s = Sample::failure(
                            t_start_us,
                            duration_us,
                            ConnectionError::Unknown(e.to_string()),
                            0,
                            is_warmup,
                        );
                        local_samples.push(s.clone());
                        let _ = tx.send(s);
                        break;
                    }
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

#[allow(clippy::too_many_arguments)]
async fn run_upload_test(
    client: &Client,
    server_url: &str,
    parallel_connections: usize,
    payload_size: usize,
    chunk_size: usize,
    duration: Duration,
    warmup: Duration,
    version: HttpVersion,
) -> Result<ThroughputResult> {
    eprintln!(
        "Starting upload test with {} payload size and {} parallel connections...",
        format_bytes(payload_size).yellow(),
        parallel_connections.to_string().yellow()
    );

    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);
    let start_time = Instant::now();

    let chunk_data = {
        let mut data = vec![0u8; chunk_size];
        rng().fill_bytes(&mut data);
        data
    };
    debug_assert!(chunk_data.len() == chunk_size, "Chunk data size mismatch");

    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let mut tasks = Vec::new();

    for _ in 0..parallel_connections {
        let client = client.clone();
        let tx = tx.clone();
        let server_url = server_url.to_string();
        let chunk_data = chunk_data.clone();

        let task = tokio::spawn(async move {
            let mut local_samples: Vec<Sample> = Vec::new();
            while start_time.elapsed() < duration {
                let upload_start = Instant::now();
                let t_start_us = offset_us(start_time, upload_start);
                let is_warmup = start_time.elapsed() < warmup;
                match upload_chunk(
                    &client,
                    &server_url,
                    payload_size,
                    chunk_data.clone(),
                    version,
                )
                .await
                {
                    Ok(bytes) => {
                        let duration_us = upload_start.elapsed().as_micros() as u64;
                        let s = Sample::success(t_start_us, duration_us, bytes, is_warmup);
                        local_samples.push(s.clone());
                        let _ = tx.send(s);
                    }
                    Err(e) => {
                        let duration_us = upload_start.elapsed().as_micros() as u64;
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

async fn download_chunk(
    client: &Client,
    server_url: &str,
    id: usize,
    payload_size: usize,
    chunk_size: usize,
    version: HttpVersion,
) -> Result<u64> {
    let response = apply_version(
        client.get(format!(
            "{server_url}/download?size={payload_size}&chunk_size={chunk_size}&id={id}"
        )),
        version,
    )
    .send()
    .await?;
    let mut total_bytes = 0u64;

    let mut stream = response.bytes_stream();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        total_bytes += chunk.len() as u64;
    }

    debug_assert!(
        payload_size.to_f64() * 0.9 <= total_bytes.to_f64()
            && total_bytes.to_f64() <= payload_size.to_f64() * 1.1,
        "Downloaded {total_bytes} bytes, expected within 10% of {payload_size} bytes"
    );

    Ok(total_bytes)
}

/// Upload `payload_size` bytes as a *single* POST whose body is a
/// stream of `chunk_size` chunks. This is the right shape for a
/// throughput test: we measure the rate at which the network can carry
/// one application-level upload, not the rate at which the client can
/// perform N back-to-back requests.
async fn upload_chunk(
    client: &Client,
    server_url: &str,
    payload_size: usize,
    chunk_data: Vec<u8>,
    version: HttpVersion,
) -> Result<u64> {
    let chunk_size = chunk_data.len();
    if chunk_size == 0 || payload_size == 0 {
        return Ok(0);
    }
    let num_chunks = payload_size.div_ceil(chunk_size);
    let chunk_template = bytes::Bytes::from(chunk_data);

    let stream = futures::stream::iter((0..num_chunks).map(move |i| {
        let bytes_already = i * chunk_size;
        let remaining = payload_size - bytes_already;
        let this_chunk = chunk_size.min(remaining);
        let bytes = if this_chunk == chunk_size {
            chunk_template.clone()
        } else {
            chunk_template.slice(0..this_chunk)
        };
        Ok::<_, std::io::Error>(bytes)
    }));

    let response = apply_version(client.post(format!("{server_url}/upload")), version)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", payload_size.to_string())
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?;

    if !response.status().is_success() {
        eyre::bail!("Upload failed with status: {}", response.status());
    }

    Ok(payload_size as u64)
}
