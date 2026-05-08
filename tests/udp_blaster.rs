//! Smoke test for the UDP blaster: latency, upload, and download
//! against a server bound on an ephemeral port.

use std::time::Duration;

use eyre::Result;
use speed_cli::TestType;
use speed_cli::performance::udp::client::run_udp_client;
use speed_cli::performance::udp::server::run_udp_server;
use speed_cli::report::{NetworkProtocol, TestResult, UdpTestConfig};
use tokio_util::sync::CancellationToken;

async fn pick_port() -> Result<u16> {
    let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    Ok(s.local_addr()?.port())
}

#[tokio::test]
async fn upload_and_download_roundtrip() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    let server = tokio::spawn(async move {
        run_udp_server(format!("127.0.0.1:{port}"), server_cancel).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let config = UdpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2, // 2-second test
        1,
        TestType::Bidirectional,
        vec![1024usize],
    )
    .with_warmup(Duration::from_millis(0))
    .with_target_rate_bps(10_000_000); // 10 Mbps - paceable on most boxes

    let report = run_udp_client(config).await?;

    let net = match &report.result {
        TestResult::Network(n) => {
            assert!(matches!(n.protocol, NetworkProtocol::Udp));
            n
        }
        _ => panic!("expected network result"),
    };

    let dl = net.download.values().next().expect("download");
    let ul = net.upload.values().next().expect("upload");
    assert!(
        dl.bytes_transferred() > 0,
        "download bytes must be > 0 (got {})",
        dl.bytes_transferred()
    );
    assert!(
        ul.bytes_transferred() > 0,
        "upload bytes must be > 0 (got {})",
        ul.bytes_transferred()
    );

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    Ok(())
}

#[tokio::test]
async fn latency_runs() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    let server =
        tokio::spawn(async move { run_udp_server(format!("127.0.0.1:{port}"), server_cancel).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let config = UdpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2,
        1,
        TestType::LatencyOnly,
        Vec::<usize>::new(),
    )
    .with_warmup(Duration::from_millis(0));

    let report = run_udp_client(config).await?;
    let net = match &report.result {
        TestResult::Network(n) => n,
        _ => panic!("expected network result"),
    };
    let latency = net.latency.as_ref().expect("latency present");
    assert!(latency.successful_count() > 0, "got at least one RTT sample");

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    Ok(())
}
