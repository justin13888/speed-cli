//! End-to-end check for the UDP `latency-under-load` stress test.
//!
//! Drives the full orchestration against a loopback server: an idle latency
//! baseline, then a high-rate latency probe running concurrently with
//! saturating download + upload load. Loopback can't reproduce real WiFi / AP
//! spikes, so this validates the *plumbing and analysis* — that both latency
//! series are captured, throughput load is applied, the bufferbloat comparison
//! is computable, and spike detection runs without panicking. Actual spikes
//! surface when the tool is pointed at a real server over WiFi.

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
async fn latency_under_load_captures_baseline_and_loaded_series() -> Result<()> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    let server =
        tokio::spawn(
            async move { run_udp_server(format!("127.0.0.1:{port}"), server_cancel).await },
        );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let config = UdpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(port),
        3, // ~1s idle baseline + ~2s under load
        1,
        TestType::LatencyUnderLoad,
        vec![1024usize],
    )
    .with_warmup(Duration::from_millis(0))
    // Pace the load so loopback stays light; the stress path itself is unchanged.
    .with_target_rate_bps(10_000_000);

    let report = run_udp_client(config).await?;

    let net = match &report.result {
        TestResult::Network(n) => {
            assert!(matches!(n.protocol, NetworkProtocol::Udp));
            n
        }
        _ => panic!("expected network result"),
    };

    // Idle baseline and under-load series are both present with samples.
    let idle = net.latency.as_ref().expect("idle baseline latency present");
    let loaded = net
        .latency_under_load
        .as_ref()
        .expect("under-load latency present");
    assert!(
        idle.successful_count() > 0,
        "idle baseline produced RTT samples"
    );
    assert!(
        loaded.successful_count() > 0,
        "under-load phase produced RTT samples"
    );

    // The saturating load was actually applied and recorded.
    let dl = net.download.values().next().expect("download recorded");
    let ul = net.upload.values().next().expect("upload recorded");
    assert!(
        dl.bytes_transferred() > 0,
        "download moved bytes under load"
    );
    assert!(ul.bytes_transferred() > 0, "upload moved bytes under load");

    // Spike detection and the bufferbloat comparison are computable (no panic).
    let spikes = loaded
        .spike_report()
        .expect("spike report on loaded series");
    assert!(
        spikes.worst_ms >= spikes.baseline_ms,
        "worst RTT is at least the baseline"
    );
    assert!(
        net.bufferbloat_inflation().is_some(),
        "idle + loaded series yield a bufferbloat comparison"
    );

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    Ok(())
}
