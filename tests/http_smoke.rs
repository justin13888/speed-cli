//! End-to-end smoke test for the HTTP server + client paths.
//!
//! Boots the strict single-protocol cleartext listeners on ephemeral
//! ports, runs short download + upload tests via the public client API,
//! and verifies that a client speaking the wrong protocol fails loudly
//! against a strict listener.

use std::time::Duration;

use eyre::Result;
use speed_cli::TestType;
use speed_cli::performance::http::HttpVersion;
use speed_cli::performance::http::client::run_http_test;
use speed_cli::performance::http::server::{HttpServerConfig, run_h2c_server, run_http1_server};
use speed_cli::report::{HttpTestConfig, NetworkProtocol, TestResult};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn server_config() -> HttpServerConfig {
    HttpServerConfig {
        enable_cors: true,
        max_upload_size: 8 * 1024 * 1024,
    }
}

fn client_config_with_type(port: u16, version: HttpVersion, test_type: TestType) -> HttpTestConfig {
    HttpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        2,
        1,
        test_type,
        vec![64 * 1024usize],
        Some(16 * 1024),
        version,
    )
    .with_warmup(Duration::from_millis(0))
}

fn client_config(port: u16, version: HttpVersion) -> HttpTestConfig {
    client_config_with_type(port, version, TestType::Bidirectional)
}

#[tokio::test]
async fn http1_download_and_upload_roundtrip() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(
            async move { run_http1_server(listener, server_config(), server_cancel).await },
        );

    tokio::time::sleep(Duration::from_millis(150)).await;

    let report = run_http_test(client_config(port, HttpVersion::HTTP1)).await?;
    let result = match &report.result {
        TestResult::Network(net) => {
            assert!(matches!(net.protocol, NetworkProtocol::Http));
            net
        }
        _ => panic!("expected NetworkTestResult"),
    };
    assert!(
        result.download.values().next().unwrap().bytes_transferred() > 0,
        "download bytes must be > 0"
    );
    assert!(
        result.upload.values().next().unwrap().bytes_transferred() > 0,
        "upload bytes must be > 0"
    );

    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join.is_ok(), "HTTP/1.1 server did not shut down within 5s");
    Ok(())
}

#[tokio::test]
async fn h2c_download_and_upload_roundtrip() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(async move { run_h2c_server(listener, server_config(), server_cancel).await });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let report = run_http_test(client_config(port, HttpVersion::H2C)).await?;
    let result = match &report.result {
        TestResult::Network(net) => net,
        _ => panic!("expected NetworkTestResult"),
    };
    assert!(
        result.download.values().next().unwrap().bytes_transferred() > 0,
        "download bytes must be > 0"
    );

    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join.is_ok(), "h2c server did not shut down within 5s");
    Ok(())
}

/// Full-duplex (`Simultaneous`) runs download and upload concurrently. A
/// regression for the warmup-tagging bug that reported zero download bytes: a
/// request tagged warmup at its *start* (rather than its completion) could be
/// the only completed request in a stream and get discarded, zeroing the row.
/// Both directions must carry bytes.
#[tokio::test]
async fn simultaneous_full_duplex_roundtrip() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(
            async move { run_http1_server(listener, server_config(), server_cancel).await },
        );

    tokio::time::sleep(Duration::from_millis(150)).await;

    let report = run_http_test(client_config_with_type(
        port,
        HttpVersion::HTTP1,
        TestType::Simultaneous,
    ))
    .await?;
    let result = match &report.result {
        TestResult::Network(net) => net,
        _ => panic!("expected NetworkTestResult"),
    };
    assert!(
        result.download.values().next().unwrap().bytes_transferred() > 0,
        "simultaneous download bytes must be > 0"
    );
    assert!(
        result.upload.values().next().unwrap().bytes_transferred() > 0,
        "simultaneous upload bytes must be > 0"
    );

    cancel.cancel();
    let join = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(join.is_ok(), "HTTP/1.1 server did not shut down within 5s");
    Ok(())
}

/// Strict separation: an HTTP/1.1 client must NOT be silently served by
/// the h2c listener — the test must fail loudly.
#[tokio::test]
async fn http1_client_against_h2c_listener_fails() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();

    let server_cancel = cancel.clone();
    let server_handle =
        tokio::spawn(async move { run_h2c_server(listener, server_config(), server_cancel).await });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let result = run_http_test(client_config(port, HttpVersion::HTTP1)).await;
    assert!(
        result.is_err(),
        "HTTP/1.1 client should fail against the h2c-only listener, got Ok"
    );

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    Ok(())
}
