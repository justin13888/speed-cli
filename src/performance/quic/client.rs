//! Raw QUIC test client.
//!
//! Opens one QUIC connection and drives the requested test over a set
//! of parallel bidirectional streams — the QUIC-idiomatic analog of the
//! raw-TCP test's parallel connections. Mirrors the raw-TCP client's
//! sample/stream/report plumbing so the two protocols are directly
//! comparable.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use colored::Colorize as _;
use eyre::{Result, eyre};
use quinn::{ClientConfig, Connection, Endpoint};
use rand::{RngCore as _, rng};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

use crate::TestType;
use crate::performance::engine::{
    LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
    measurement_duration_us, offset_us,
};
use crate::performance::handshake::client_hello_io;
use crate::performance::quic::QUIC_RAW_ALPN;
use crate::report::{
    ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, PeerIdentity,
    QuicTestConfig, Sample, StreamSamples, TestReport, ThroughputResult,
};

/// Certificate verifier that accepts any server certificate. This
/// mirrors `reqwest`'s `danger_accept_invalid_certs(true)` used
/// everywhere else in this tool: speed-cli targets self-signed test
/// servers, so chain validation is intentionally skipped.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn client_config() -> Result<ClientConfig> {
    let mut crypto =
        rustls::ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| eyre!("raw-QUIC client TLS setup: {e}"))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
            .with_no_client_auth();
    crypto.alpn_protocols = vec![QUIC_RAW_ALPN.to_vec()];

    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .map_err(|e| eyre!("raw-QUIC client crypto config: {e}"))?;
    Ok(ClientConfig::new(Arc::new(quic)))
}

async fn connect(server: &str, port: u16) -> Result<(Endpoint, Connection)> {
    let addr: SocketAddr = tokio::net::lookup_host((server, port))
        .await
        .map_err(|e| eyre!("raw-QUIC: resolving {server}:{port}: {e}"))?
        .next()
        .ok_or_else(|| eyre!("raw-QUIC: no address for {server}:{port}"))?;

    let bind: SocketAddr = if addr.is_ipv6() {
        SocketAddr::from(([0u16; 8], 0))
    } else {
        SocketAddr::from(([0u8; 4], 0))
    };
    let mut endpoint =
        Endpoint::client(bind).map_err(|e| eyre!("raw-QUIC: client endpoint: {e}"))?;
    endpoint.set_default_client_config(client_config()?);

    let conn = endpoint
        .connect(addr, "localhost")
        .map_err(|e| eyre!("raw-QUIC: connect to {addr}: {e}"))?
        .await
        .map_err(|e| eyre!("raw-QUIC: handshake with {addr}: {e}"))?;
    Ok((endpoint, conn))
}

pub async fn run_quic_client(config: QuicTestConfig) -> Result<TestReport> {
    tracing::info!(
        "{}",
        format!(
            "Starting raw-QUIC test to server {}:{}...",
            config.server.cyan(),
            config.port
        )
        .green()
        .bold()
    );

    let (endpoint, conn) = connect(&config.server, config.port).await?;
    let remote_addr = conn.remote_address();

    // Best-effort identity handshake on a dedicated bidi stream.
    let server_identity = quic_hello(&conn).await;

    let start_time = Utc::now();
    let mut result = NetworkTestResult::new_quic().with_accounting(config.accounting);

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = measure_latency(&conn, &config).await?;
        }
        TestType::Download => {
            for &size in &config.payload_sizes {
                result
                    .download
                    .insert(size, run_download(&conn, &config).await?);
            }
        }
        TestType::Upload => {
            for &size in &config.payload_sizes {
                result
                    .upload
                    .insert(size, run_upload(&conn, &config, size).await?);
            }
        }
        TestType::Bidirectional => {
            for &size in &config.payload_sizes {
                result
                    .download
                    .insert(size, run_download(&conn, &config).await?);
                result
                    .upload
                    .insert(size, run_upload(&conn, &config, size).await?);
            }
        }
        TestType::Simultaneous => {
            for &size in &config.payload_sizes {
                let (dl, ul) = tokio::join!(
                    run_download(&conn, &config),
                    run_upload(&conn, &config, size)
                );
                result.download.insert(size, dl?);
                result.upload.insert(size, ul?);
            }
        }
        TestType::FullDuplex => {
            for &size in &config.payload_sizes {
                let (dl, ul) = run_full_duplex(&conn, &config, size).await?;
                result.download.insert(size, dl);
                result.upload.insert(size, ul);
            }
        }
    }

    conn.close(0u32.into(), b"done");
    endpoint.wait_idle().await;

    let mut report: TestReport = (start_time, config, result).into();
    report.peers.client.remote_addr = Some(remote_addr);
    if let Some((identity, observed)) = server_identity {
        report.peers.server.identity = Some(identity);
        report.peers.server.observed_client_addr = Some(observed);
    }
    Ok(report)
}

/// Best-effort identity handshake. Returns `None` if the server didn't
/// answer the `'H'` exchange.
async fn quic_hello(conn: &Connection) -> Option<(PeerIdentity, SocketAddr)> {
    let (mut send, mut recv) = conn.open_bi().await.ok()?;
    let result = tokio::time::timeout(
        crate::performance::handshake::HELLO_TIMEOUT,
        client_hello_io(&mut recv, &mut send, &PeerIdentity::local()),
    )
    .await
    .ok()?
    .ok()?;
    let _ = send.finish();
    Some((result.server_identity, result.observed_client_addr))
}

async fn run_download(conn: &Connection, config: &QuicTestConfig) -> Result<ThroughputResult> {
    tracing::info!(
        "Starting raw-QUIC download over {} streams...",
        config.parallel_connections.to_string().yellow()
    );
    let duration = config.duration;
    let warmup = config.warmup;
    let buf_size = config.read_buffer_size;

    let pb = create_progress_bar(ProgressBarType::Download, duration);
    let start = Instant::now();
    let (collector, tx) = ThroughputStatsCollector::new(pb.clone(), start, duration);

    let mut tasks = Vec::new();
    for _ in 0..config.parallel_connections {
        let conn = conn.clone();
        let tx = tx.clone();
        tasks.push(tokio::spawn(async move {
            let mut samples: Vec<Sample> = Vec::new();
            let (mut send, mut recv) = match conn.open_bi().await {
                Ok(s) => s,
                Err(e) => {
                    samples.push(Sample::failure(
                        0,
                        0,
                        ConnectionError::ConnectionFailed(format!("open_bi: {e}")),
                        0,
                        false,
                    ));
                    return samples;
                }
            };
            if send.write_all(b"D").await.is_err() {
                return samples;
            }
            let _ = send.finish();
            let mut buf = vec![0u8; buf_size];
            while start.elapsed() < duration {
                let op_start = Instant::now();
                let t_start_us = offset_us(start, op_start);
                let is_warmup = start.elapsed() < warmup;
                match recv.read(&mut buf).await {
                    Ok(Some(n)) => {
                        let s = Sample::success(
                            t_start_us,
                            op_start.elapsed().as_micros() as u64,
                            n as u64,
                            is_warmup,
                        );
                        samples.push(s.clone());
                        let _ = tx.send(s);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let s = Sample::failure(
                            t_start_us,
                            op_start.elapsed().as_micros() as u64,
                            ConnectionError::Unknown(e.to_string()),
                            0,
                            is_warmup,
                        );
                        samples.push(s.clone());
                        let _ = tx.send(s);
                        break;
                    }
                }
            }
            samples
        }));
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = collector
        .finish(pb, "raw-QUIC download complete".to_string())
        .await;
    Ok(into_result(results, start, warmup))
}

async fn run_upload(
    conn: &Connection,
    config: &QuicTestConfig,
    payload_size: usize,
) -> Result<ThroughputResult> {
    tracing::info!(
        "Starting raw-QUIC upload over {} streams...",
        config.parallel_connections.to_string().yellow()
    );
    let duration = config.duration;
    let warmup = config.warmup;

    let payload = {
        let mut d = vec![0u8; payload_size.max(1)];
        rng().fill_bytes(&mut d);
        Arc::new(d)
    };

    let pb = create_progress_bar(ProgressBarType::Upload, duration);
    let start = Instant::now();
    let (collector, tx) = ThroughputStatsCollector::new(pb.clone(), start, duration);

    let mut tasks = Vec::new();
    for _ in 0..config.parallel_connections {
        let conn = conn.clone();
        let tx = tx.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            let mut samples: Vec<Sample> = Vec::new();
            let (mut send, _recv) = match conn.open_bi().await {
                Ok(s) => s,
                Err(e) => {
                    samples.push(Sample::failure(
                        0,
                        0,
                        ConnectionError::ConnectionFailed(format!("open_bi: {e}")),
                        0,
                        false,
                    ));
                    return samples;
                }
            };
            if send.write_all(b"U").await.is_err() {
                return samples;
            }
            while start.elapsed() < duration {
                let op_start = Instant::now();
                let t_start_us = offset_us(start, op_start);
                let is_warmup = start.elapsed() < warmup;
                match send.write_all(&payload).await {
                    Ok(()) => {
                        let s = Sample::success(
                            t_start_us,
                            op_start.elapsed().as_micros() as u64,
                            payload.len() as u64,
                            is_warmup,
                        );
                        samples.push(s.clone());
                        let _ = tx.send(s);
                    }
                    Err(e) => {
                        let s = Sample::failure(
                            t_start_us,
                            op_start.elapsed().as_micros() as u64,
                            ConnectionError::TransferFailed(e.to_string()),
                            0,
                            is_warmup,
                        );
                        samples.push(s.clone());
                        let _ = tx.send(s);
                        break;
                    }
                }
            }
            let _ = send.finish();
            samples
        }));
    }

    let results = futures::future::join_all(tasks).await;
    drop(tx);
    let _ = collector
        .finish(pb, "raw-QUIC upload complete".to_string())
        .await;
    Ok(into_result(results, start, warmup))
}

async fn run_full_duplex(
    conn: &Connection,
    config: &QuicTestConfig,
    payload_size: usize,
) -> Result<(ThroughputResult, ThroughputResult)> {
    tracing::info!(
        "Starting raw-QUIC full-duplex over {} streams...",
        config.parallel_connections.to_string().yellow()
    );
    let duration = config.duration;
    let warmup = config.warmup;
    let buf_size = config.read_buffer_size;

    let payload = {
        let mut d = vec![0u8; payload_size.max(1)];
        rng().fill_bytes(&mut d);
        Arc::new(d)
    };

    let dl_pb = create_progress_bar(ProgressBarType::Download, duration);
    let ul_pb = create_progress_bar(ProgressBarType::Upload, duration);
    let start = Instant::now();
    let (dl_collector, dl_tx) = ThroughputStatsCollector::new(dl_pb.clone(), start, duration);
    let (ul_collector, ul_tx) = ThroughputStatsCollector::new(ul_pb.clone(), start, duration);

    let mut tasks: Vec<tokio::task::JoinHandle<(Vec<Sample>, Vec<Sample>)>> = Vec::new();
    for _ in 0..config.parallel_connections {
        let conn = conn.clone();
        let dl_tx = dl_tx.clone();
        let ul_tx = ul_tx.clone();
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            let mut dl: Vec<Sample> = Vec::new();
            let mut ul: Vec<Sample> = Vec::new();
            let (mut send, mut recv) = match conn.open_bi().await {
                Ok(s) => s,
                Err(_) => return (dl, ul),
            };
            if send.write_all(b"F").await.is_err() {
                return (dl, ul);
            }
            let read_fut = async {
                let mut buf = vec![0u8; buf_size];
                while start.elapsed() < duration {
                    let op = Instant::now();
                    let t = offset_us(start, op);
                    let w = start.elapsed() < warmup;
                    match recv.read(&mut buf).await {
                        Ok(Some(n)) => {
                            let s =
                                Sample::success(t, op.elapsed().as_micros() as u64, n as u64, w);
                            dl.push(s.clone());
                            let _ = dl_tx.send(s);
                        }
                        _ => break,
                    }
                }
            };
            let write_fut = async {
                while start.elapsed() < duration {
                    let op = Instant::now();
                    let t = offset_us(start, op);
                    let w = start.elapsed() < warmup;
                    match send.write_all(&payload).await {
                        Ok(()) => {
                            let s = Sample::success(
                                t,
                                op.elapsed().as_micros() as u64,
                                payload.len() as u64,
                                w,
                            );
                            ul.push(s.clone());
                            let _ = ul_tx.send(s);
                        }
                        Err(_) => break,
                    }
                }
                let _ = send.finish();
            };
            tokio::join!(read_fut, write_fut);
            (dl, ul)
        }));
    }

    let results = futures::future::join_all(tasks).await;
    drop(dl_tx);
    drop(ul_tx);
    let _ = dl_collector
        .finish(dl_pb, "raw-QUIC full-duplex download complete".to_string())
        .await;
    let _ = ul_collector
        .finish(ul_pb, "raw-QUIC full-duplex upload complete".to_string())
        .await;

    let mut dl_streams = Vec::with_capacity(results.len());
    let mut ul_streams = Vec::with_capacity(results.len());
    for (idx, joined) in results.into_iter().enumerate() {
        let (dl, ul) = joined.unwrap_or_else(|_| (Vec::new(), Vec::new()));
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
    let end = Instant::now();
    let dur_us = measurement_duration_us(start, end, warmup);
    let ts = chrono::Utc::now();
    Ok((
        ThroughputResult {
            streams: dl_streams,
            total_duration_us: dur_us,
            timestamp: ts,
            udp_stats: None,
            udp_series: Vec::new(),
            udp_series_window_us: 0,
        },
        ThroughputResult {
            streams: ul_streams,
            total_duration_us: dur_us,
            timestamp: ts,
            udp_stats: None,
            udp_series: Vec::new(),
            udp_series_window_us: 0,
        },
    ))
}

async fn measure_latency(
    conn: &Connection,
    config: &QuicTestConfig,
) -> Result<Option<LatencyResult>> {
    let duration = config.duration;
    let warmup = config.warmup;
    tracing::info!("Measuring raw-QUIC in-stream RTT for {duration:?}...");

    let pb = create_progress_bar(ProgressBarType::Latency, duration);
    let start = Instant::now();
    let (collector, tx) = LatencyStatsCollector::new(pb.clone(), start, duration);

    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| eyre!("raw-QUIC latency: open_bi: {e}"))?;
    send.write_all(b"P")
        .await
        .map_err(|e| eyre!("raw-QUIC latency: send 'P': {e}"))?;

    let mut send_buf = [0u8; 8];
    let mut recv_buf = [0u8; 8];
    while start.elapsed() < duration {
        let in_warmup = start.elapsed() < warmup;
        let probe = Instant::now();
        let t_start_us = offset_us(start, probe);
        let nonce: u64 = probe.elapsed().as_micros() as u64;
        send_buf.copy_from_slice(&nonce.to_le_bytes());

        let measurement = match send.write_all(&send_buf).await {
            Ok(()) => {
                match tokio::time::timeout(Duration::from_secs(2), recv.read_exact(&mut recv_buf))
                    .await
                {
                    Ok(Ok(())) => {
                        LatencyMeasurement::success(t_start_us, probe.elapsed().as_micros() as u64)
                    }
                    _ => LatencyMeasurement::dropped(t_start_us),
                }
            }
            Err(_) => LatencyMeasurement::dropped(t_start_us),
        };
        if !in_warmup {
            let _ = tx.send(measurement);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = send.finish();
    drop(tx);

    let measurements = collector
        .finish(pb, "raw-QUIC latency measurement complete".to_string())
        .await;
    if measurements.is_empty() {
        return Ok(None);
    }
    Ok(Some(LatencyResult {
        measurements,
        timestamp: chrono::Utc::now(),
    }))
}

/// Fold joined per-stream sample vectors into a [`ThroughputResult`].
fn into_result(
    results: Vec<std::result::Result<Vec<Sample>, tokio::task::JoinError>>,
    start: Instant,
    warmup: Duration,
) -> ThroughputResult {
    let mut streams = Vec::with_capacity(results.len());
    for (idx, joined) in results.into_iter().enumerate() {
        let samples = joined.unwrap_or_default();
        streams.push(StreamSamples {
            stream_id: idx as u32,
            start_offset_us: samples.first().map(|s| s.t_start_us).unwrap_or(0),
            samples,
        });
    }
    ThroughputResult {
        streams,
        total_duration_us: measurement_duration_us(start, Instant::now(), warmup),
        timestamp: chrono::Utc::now(),
        udp_stats: None,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
    }
}
