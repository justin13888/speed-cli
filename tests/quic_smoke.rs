//! End-to-end smoke test for the raw-QUIC server + client path.
//!
//! Binds a raw-QUIC endpoint on an ephemeral port, runs a short
//! download + upload, and asserts non-zero throughput.

use std::time::Duration;

use eyre::Result;
use speed_cli::performance::quic::client::run_quic_client;
use speed_cli::performance::quic::server::{QuicServerConfig, bind_quic, run_quic_server};
use speed_cli::report::{NetworkProtocol, QuicTestConfig, TestConfig, TestResult};
use speed_cli::utils::tls::TlsMaterial;
use speed_cli::{CongestionAlgorithm, TestType};
use tokio_util::sync::CancellationToken;

/// Bind a server and drive one bidirectional client run, both sides
/// using `congestion`.
async fn roundtrip(congestion: CongestionAlgorithm) -> Result<()> {
    let cfg = QuicServerConfig {
        tls: TlsMaterial::self_signed()?,
        buffer_size: 65536,
        congestion,
    };
    let (endpoint, port) = bind_quic("127.0.0.1:0".parse().unwrap(), &cfg)?;

    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(async move { run_quic_server(endpoint, cfg, server_cancel).await });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let config = QuicTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2,
        2,
        TestType::Bidirectional,
        vec![65536usize],
    )
    .with_warmup(Duration::from_millis(0))
    .with_congestion(congestion);

    let report = run_quic_client(config).await?;
    let result = match &report.result {
        TestResult::Network(net) => {
            assert!(matches!(net.protocol, NetworkProtocol::Quic));
            net
        }
        _ => panic!("expected NetworkTestResult"),
    };
    assert!(
        result.download.values().next().unwrap().bytes_transferred() > 0,
        "raw-QUIC download bytes must be > 0"
    );
    assert!(
        result.upload.values().next().unwrap().bytes_transferred() > 0,
        "raw-QUIC upload bytes must be > 0"
    );
    // The report must record which controller actually ran.
    match &report.config {
        TestConfig::Quic(c) => assert_eq!(c.congestion, congestion),
        _ => panic!("expected Quic config"),
    }

    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join.is_ok(), "raw-QUIC server did not shut down within 5s");
    Ok(())
}

#[tokio::test]
async fn quic_download_and_upload_roundtrip() -> Result<()> {
    roundtrip(CongestionAlgorithm::Cubic).await
}

#[tokio::test]
async fn quic_bbr_roundtrip() -> Result<()> {
    roundtrip(CongestionAlgorithm::Bbr).await
}
