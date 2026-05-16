//! Comprehensive end-to-end suite. Performs the control handshake,
//! then runs every protocol the server advertises against it, stitches
//! the per-protocol [`TestReport`]s into a single [`SuiteReport`], and
//! prints / exports a unified result.
//!
//! Because the suite is handshake-driven, the manifest *is* the
//! availability map: a phase runs if and only if the server advertised
//! that transport. There is no separate port-probing step.

use std::time::Duration;

use colored::Colorize as _;
use eyre::Result;

use crate::TestType;
use crate::control::{Handshake, TestTransport, perform_handshake};
use crate::performance::http::HttpVersion;
use crate::performance::http::client::run_http_test;
use crate::performance::quic::client::run_quic_client;
use crate::performance::tcp::client::run_tcp_client;
use crate::performance::udp::client::run_udp_client;
use crate::report::{
    HttpTestConfig, QuicTestConfig, SuiteReport, TcpTestConfig, ThroughputAccounting,
    UdpTestConfig,
};

/// User-facing knobs for the suite.
#[derive(Debug, Clone)]
pub struct SuiteConfig {
    pub server: String,
    /// Control endpoint port — the only port the user supplies.
    pub control_port: u16,
    /// Wall-clock per phase.
    pub phase_duration: Duration,
    /// Warmup window inside each phase (counts against `phase_duration`).
    pub warmup: Duration,
    /// Parallel TCP connections / HTTP streams / QUIC streams.
    pub connections: usize,
    /// Target rate for the UDP throughput phase, in Mbps. 0 = saturate.
    pub udp_target_rate_mbps: u64,
    pub accounting: ThroughputAccounting,
    /// When false, TLS phases (HTTP/2-TLS, HTTP/3) are force-skipped
    /// even if the server advertises them.
    pub include_tls: bool,
}

impl SuiteConfig {
    pub fn new(server: String) -> Self {
        Self {
            server,
            control_port: crate::constants::DEFAULT_CONTROL_PORT,
            phase_duration: Duration::from_secs(8),
            warmup: Duration::from_secs(1),
            connections: 4,
            udp_target_rate_mbps: 100,
            accounting: ThroughputAccounting::Goodput,
            include_tls: true,
        }
    }
}

/// Drive the whole suite. The handshake runs first: a protocol-version
/// mismatch aborts the entire suite immediately (no point running
/// phases against an incompatible server).
pub async fn run_suite(cfg: SuiteConfig) -> Result<SuiteReport> {
    let handshake = perform_handshake(&cfg.server, cfg.control_port).await?;
    let mut suite = SuiteReport::new(cfg.server.clone());

    let dur = cfg.phase_duration.as_secs();

    // ── TCP ─────────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::TcpRaw).is_some() {
        run_phase(&mut suite, "tcp/latency", run_tcp_phase(&handshake, &cfg, TestType::LatencyOnly, dur)).await;
        run_phase(&mut suite, "tcp/bidirectional", run_tcp_phase(&handshake, &cfg, TestType::Bidirectional, dur)).await;
        run_phase(&mut suite, "tcp/full-duplex", run_tcp_phase(&handshake, &cfg, TestType::FullDuplex, dur)).await;
    } else {
        suite.skip("tcp/*", "TCP listener not advertised by server");
    }

    // ── UDP ─────────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::UdpBlaster).is_some() {
        run_phase(&mut suite, "udp/latency", run_udp_phase(&handshake, &cfg, TestType::LatencyOnly, dur)).await;
        run_phase(&mut suite, "udp/bidirectional", run_udp_phase(&handshake, &cfg, TestType::Bidirectional, dur)).await;
    } else {
        suite.skip("udp/*", "UDP listener not advertised by server");
    }

    // ── Raw QUIC ────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::QuicRaw).is_some() {
        run_phase(&mut suite, "quic/latency", run_quic_phase(&handshake, &cfg, TestType::LatencyOnly, dur)).await;
        run_phase(&mut suite, "quic/bidirectional", run_quic_phase(&handshake, &cfg, TestType::Bidirectional, dur)).await;
        run_phase(&mut suite, "quic/full-duplex", run_quic_phase(&handshake, &cfg, TestType::FullDuplex, dur)).await;
    } else {
        suite.skip("quic/*", "raw-QUIC listener not advertised by server");
    }

    // ── HTTP/1.1 ────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::Http1).is_some() {
        run_phase(&mut suite, "http1/bidirectional", run_http_phase(&handshake, &cfg, HttpVersion::HTTP1, TestTransport::Http1, dur)).await;
    } else {
        suite.skip("http1/*", "HTTP/1.1 listener not advertised by server");
    }

    // ── h2c ─────────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::H2c).is_some() {
        run_phase(&mut suite, "h2c/bidirectional", run_http_phase(&handshake, &cfg, HttpVersion::H2C, TestTransport::H2c, dur)).await;
    } else {
        suite.skip("h2c/*", "h2c listener not advertised by server");
    }

    // ── HTTP/2 over TLS ─────────────────────────────────────────────
    if !cfg.include_tls {
        suite.skip("http2/*", "TLS phases skipped (--no-tls)");
    } else if handshake.manifest.listener(TestTransport::Http2Tls).is_some() {
        run_phase(&mut suite, "http2/bidirectional", run_http_phase(&handshake, &cfg, HttpVersion::HTTP2, TestTransport::Http2Tls, dur)).await;
    } else {
        suite.skip("http2/*", "HTTP/2-TLS listener not advertised by server");
    }

    // ── HTTP/3 ──────────────────────────────────────────────────────
    if !cfg.include_tls {
        suite.skip("http3/*", "TLS phases skipped (--no-tls)");
    } else if handshake.manifest.listener(TestTransport::Http3).is_some() {
        run_phase(&mut suite, "http3/bidirectional", run_http_phase(&handshake, &cfg, HttpVersion::HTTP3, TestTransport::Http3, dur)).await;
    } else {
        suite.skip("http3/*", "HTTP/3 listener not advertised by server");
    }

    suite.finalize();
    Ok(suite)
}

async fn run_phase<F>(suite: &mut SuiteReport, label: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<crate::report::TestReport>>,
{
    eprintln!(
        "{}",
        format!("\n── Suite phase: {label} ──").bright_magenta().bold()
    );
    match fut.await {
        Ok(report) => suite.record(label, report),
        Err(e) => {
            tracing::warn!(phase = label, error = %e, "suite phase failed; continuing");
            suite.skip(label, e.to_string());
        }
    }
}

async fn run_tcp_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    test_type: TestType,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(TestTransport::TcpRaw)?;
    let payload_sizes: Vec<usize> = if matches!(test_type, TestType::LatencyOnly) {
        Vec::new()
    } else {
        vec![65536]
    };
    let connections = if matches!(test_type, TestType::LatencyOnly) {
        1
    } else {
        cfg.connections
    };
    let conf = TcpTestConfig::new(host, Some(port), duration, connections, test_type, payload_sizes)
        .with_warmup(cfg.warmup)
        .with_accounting(cfg.accounting);
    run_tcp_client(conf).await
}

async fn run_udp_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    test_type: TestType,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(TestTransport::UdpBlaster)?;
    let payload_sizes: Vec<usize> = if matches!(test_type, TestType::LatencyOnly) {
        Vec::new()
    } else {
        vec![1200]
    };
    let conf = UdpTestConfig::new(host, Some(port), duration, 1, test_type, payload_sizes)
        .with_warmup(cfg.warmup)
        .with_accounting(cfg.accounting)
        .with_target_rate_bps(cfg.udp_target_rate_mbps.saturating_mul(1_000_000));
    run_udp_client(conf).await
}

async fn run_quic_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    test_type: TestType,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(TestTransport::QuicRaw)?;
    let payload_sizes: Vec<usize> = if matches!(test_type, TestType::LatencyOnly) {
        Vec::new()
    } else {
        vec![65536]
    };
    let connections = if matches!(test_type, TestType::LatencyOnly) {
        1
    } else {
        cfg.connections
    };
    let conf = QuicTestConfig::new(host, Some(port), duration, connections, test_type, payload_sizes)
        .with_warmup(cfg.warmup)
        .with_accounting(cfg.accounting);
    run_quic_client(conf).await
}

async fn run_http_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    version: HttpVersion,
    transport: TestTransport,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(transport)?;
    // Smaller payload than the standalone HTTP test so the suite finishes
    // in minutes; 8 MB is large enough not to be setup-dominated.
    let payload_sizes: Vec<usize> = vec![8 * 1024 * 1024];
    let conf = HttpTestConfig::new(
        host,
        Some(port),
        duration,
        cfg.connections,
        TestType::Bidirectional,
        payload_sizes,
        Some(1024 * 1024),
        version,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting);
    run_http_test(conf).await
}
