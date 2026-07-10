//! End-to-end smoke test for the HTTP/3 server + client path.
//!
//! Binds an HTTP/3 QUIC endpoint on an ephemeral port, runs a short
//! download + upload via the reqwest-based client, and asserts non-zero
//! throughput.

use std::time::Duration;

use eyre::Result;
use speed_cli::performance::http::HttpVersion;
use speed_cli::performance::http::client::run_http_test;
use speed_cli::performance::http::h3_server::{Http3ServerConfig, bind_h3, run_h3_server};
use speed_cli::report::{HttpTestConfig, TestConfig, TestResult};
use speed_cli::utils::tls::TlsMaterial;
use speed_cli::{CongestionAlgorithm, TestType};
use tokio_util::sync::CancellationToken;

/// Bind an HTTP/3 server and drive one bidirectional client run, both
/// sides using `congestion`.
async fn roundtrip(congestion: CongestionAlgorithm) -> Result<()> {
    let cfg = Http3ServerConfig {
        max_upload_size: 8 * 1024 * 1024,
        tls: TlsMaterial::self_signed()?,
        congestion,
    };
    let (endpoint, port) = bind_h3("127.0.0.1:0".parse().unwrap(), &cfg)?;

    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(async move { run_h3_server(endpoint, cfg, server_cancel).await });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let config = HttpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2,
        1,
        TestType::Bidirectional,
        vec![64 * 1024usize],
        Some(16 * 1024),
        HttpVersion::HTTP3,
    )
    .with_warmup(Duration::from_millis(0))
    .with_congestion(congestion);

    let report = run_http_test(config).await?;
    let result = match &report.result {
        TestResult::Network(net) => net,
        _ => panic!("expected NetworkTestResult"),
    };
    assert!(
        result.download.values().next().unwrap().bytes_transferred() > 0,
        "HTTP/3 download bytes must be > 0"
    );
    assert!(
        result.upload.values().next().unwrap().bytes_transferred() > 0,
        "HTTP/3 upload bytes must be > 0"
    );
    match &report.config {
        TestConfig::Http(c) => assert_eq!(c.congestion, congestion),
        _ => panic!("expected Http config"),
    }

    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join.is_ok(), "HTTP/3 server did not shut down within 5s");
    Ok(())
}

#[tokio::test]
async fn http3_download_and_upload_roundtrip() -> Result<()> {
    roundtrip(CongestionAlgorithm::Cubic).await
}

#[tokio::test]
async fn http3_bbr_roundtrip() -> Result<()> {
    roundtrip(CongestionAlgorithm::Bbr).await
}
