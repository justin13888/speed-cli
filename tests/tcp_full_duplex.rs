//! Verify the full-duplex TCP test type drives both directions on the
//! same connection and produces non-zero results in both.

use std::time::Duration;

use eyre::Result;
use speed_cli::TestType;
use speed_cli::performance::tcp::client::run_tcp_client;
use speed_cli::performance::tcp::server::run_tcp_server;
use speed_cli::report::{NetworkProtocol, TcpTestConfig, TestResult};
use tokio_util::sync::CancellationToken;

async fn pick_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

#[tokio::test]
async fn full_duplex_drives_both_directions() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle = tokio::spawn(async move {
        let addr = format!("127.0.0.1:{port}");
        run_tcp_server(addr, server_cancel).await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let config = TcpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2, // 2-second test
        1, // single connection
        TestType::FullDuplex,
        vec![8192usize],
    )
    .with_warmup(Duration::from_millis(0));

    let report = run_tcp_client(config).await?;

    let result = match &report.result {
        TestResult::Network(net) => {
            assert!(matches!(net.protocol, NetworkProtocol::Tcp));
            net
        }
        _ => panic!("expected NetworkTestResult, got Simple"),
    };

    let dl = result.download.values().next().expect("download result");
    let ul = result.upload.values().next().expect("upload result");
    assert!(
        dl.bytes_transferred() > 0,
        "full-duplex download bytes must be > 0 (got {})",
        dl.bytes_transferred()
    );
    assert!(
        ul.bytes_transferred() > 0,
        "full-duplex upload bytes must be > 0 (got {})",
        ul.bytes_transferred()
    );

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(35), server_handle).await;
    Ok(())
}
