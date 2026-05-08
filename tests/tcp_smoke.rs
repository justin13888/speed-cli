//! End-to-end smoke test for the TCP server + client path.
//!
//! Binds the server on an ephemeral port, runs a 1-second download and a
//! 1-second upload through the public client API, asserts non-zero
//! throughput, and verifies the server shuts down cleanly when its
//! cancellation token fires.

use std::time::Duration;

use eyre::Result;
use speed_cli::TestType;
use speed_cli::performance::tcp::client::run_tcp_client;
use speed_cli::performance::tcp::server::run_tcp_server;
use speed_cli::report::{NetworkProtocol, TcpTestConfig, TestResult};
use tokio_util::sync::CancellationToken;

/// Bind and immediately drop a listener to discover an unused port.
/// There is a small TOCTOU race vs. another process grabbing the port,
/// but for local test runs this is fine.
async fn pick_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

#[tokio::test]
async fn tcp_download_and_upload_roundtrip() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle = tokio::spawn(async move {
        let addr = format!("127.0.0.1:{port}");
        run_tcp_server(addr, server_cancel).await
    });

    // Give the server a moment to bind. (`run_tcp_server` only calls
    // `TcpListener::bind` after spawning; without a brief yield the client
    // can race ahead and connection-refused.)
    tokio::time::sleep(Duration::from_millis(100)).await;

    let config = TcpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2, // 2-second test (1s warmup + 1s measurement after the override)
        1, // single connection
        TestType::Bidirectional,
        vec![8192usize], // single payload size
    )
    .with_warmup(Duration::from_millis(0));

    let report = run_tcp_client(config).await?;

    // The report should contain at least one download and one upload entry.
    let result = match &report.result {
        TestResult::Network(net) => {
            assert!(matches!(net.protocol, NetworkProtocol::Tcp));
            net
        }
        _ => panic!("expected NetworkTestResult, got Simple"),
    };
    assert!(
        !result.download.is_empty(),
        "download results should be present"
    );
    assert!(
        !result.upload.is_empty(),
        "upload results should be present"
    );

    let download = result.download.values().next().expect("download result");
    let upload = result.upload.values().next().expect("upload result");
    assert!(
        download.bytes_transferred() > 0,
        "download bytes must be > 0, got {}",
        download.bytes_transferred()
    );
    assert!(
        upload.bytes_transferred() > 0,
        "upload bytes must be > 0, got {}",
        upload.bytes_transferred()
    );

    cancel.cancel();
    let join_res = tokio::time::timeout(Duration::from_secs(35), server_handle).await;
    assert!(
        join_res.is_ok(),
        "server did not shut down within 35s of cancel"
    );

    Ok(())
}
