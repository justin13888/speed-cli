//! End-to-end smoke test for the HTTP server + client path (HTTP/1.1 only).
//!
//! Boots the plaintext HTTP server on an ephemeral port, runs a 1-second
//! download + upload via the public client API, asserts non-zero throughput,
//! and verifies graceful shutdown.

use std::time::Duration;

use eyre::Result;
use speed_cli::TestType;
use speed_cli::performance::http::HttpVersion;
use speed_cli::performance::http::client::run_http_test;
use speed_cli::performance::http::server::{HttpServerConfig, run_http_server};
use speed_cli::report::{HttpTestConfig, NetworkProtocol, TestResult};
use tokio_util::sync::CancellationToken;

async fn pick_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

#[tokio::test]
async fn http1_download_and_upload_roundtrip() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle = tokio::spawn(async move {
        run_http_server(
            HttpServerConfig {
                bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
                enable_cors: true,
                max_upload_size: 8 * 1024 * 1024,
            },
            server_cancel,
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let config = HttpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        1, // 1-second test
        1, // single connection
        TestType::Bidirectional,
        vec![64 * 1024usize], // 64 KB payload
        Some(16 * 1024),      // 16 KB chunk
        HttpVersion::HTTP1,
    );

    let report = run_http_test(config).await?;

    let result = match &report.result {
        TestResult::Network(net) => {
            assert!(matches!(net.protocol, NetworkProtocol::Http));
            net
        }
        _ => panic!("expected NetworkTestResult, got Simple"),
    };
    let download = result.download.values().next().expect("download result");
    let upload = result.upload.values().next().expect("upload result");
    assert!(download.bytes_transferred() > 0, "download bytes must be > 0");
    assert!(upload.bytes_transferred() > 0, "upload bytes must be > 0");

    cancel.cancel();
    let join_res = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join_res.is_ok(), "HTTP server did not shut down within 5s");

    Ok(())
}
