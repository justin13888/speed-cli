//! Comprehensive end-to-end suite. Runs each protocol the tool
//! supports against the same server, stitches the per-protocol
//! [`TestReport`]s into a single [`SuiteReport`], and prints / exports
//! a unified result. The shape is deliberately opinionated - one
//! command produces one report covering every protocol - because
//! that's the workflow that the CLI's "test between two arbitrary
//! machines" mission asks for.

use std::time::Duration;

use colored::Colorize as _;
use eyre::Result;
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::TestType;
use crate::performance::http::HttpVersion;
use crate::performance::http::client::run_http_test;
use crate::performance::tcp::client::run_tcp_client;
use crate::performance::udp::client::run_udp_client;
use crate::report::{
    HttpTestConfig, SuiteReport, TcpTestConfig, ThroughputAccounting, UdpTestConfig,
};

/// User-facing knobs for the suite. Defaults are calibrated to take
/// a couple of minutes total against a healthy LAN; users testing on a
/// flaky link should bump `phase_duration` so percentiles stabilize.
#[derive(Debug, Clone)]
pub struct SuiteConfig {
    pub server: String,
    /// TCP/UDP port (defaults to 5201).
    pub tcp_udp_port: u16,
    /// HTTP (cleartext) port (defaults to 8080).
    pub http_port: u16,
    /// HTTPS port (defaults to 8443).
    pub https_port: u16,
    /// Wall-clock per phase. Bigger = more samples, longer total run.
    pub phase_duration: Duration,
    /// Warmup window inside each phase (counts against phase_duration).
    pub warmup: Duration,
    /// Parallel TCP connections / HTTP streams. UDP runs single-stream
    /// today (multi-stream UDP is on the roadmap).
    pub connections: usize,
    /// Target rate for the UDP throughput phase, in Mbps. 0 = saturate
    /// (note: pacing above ~100 Mbps is approximate; see UDP docs).
    pub udp_target_rate_mbps: u64,
    pub accounting: ThroughputAccounting,
    /// HTTPS phase needs TLS but we always trust self-signed; this
    /// flag merely suppresses the HTTPS phase entirely if the user
    /// knows the server has no TLS certificate available.
    pub include_tls: bool,
}

impl SuiteConfig {
    pub fn new(server: String) -> Self {
        Self {
            server,
            tcp_udp_port: 5201,
            http_port: 8080,
            https_port: 8443,
            phase_duration: Duration::from_secs(8),
            warmup: Duration::from_secs(1),
            connections: 4,
            udp_target_rate_mbps: 100,
            accounting: ThroughputAccounting::Goodput,
            include_tls: true,
        }
    }
}

/// Drive the whole suite. On a per-phase failure (server unreachable,
/// pre-flight rejected, etc.) we record the reason and continue so
/// the user still gets numbers from the protocols that worked.
pub async fn run_suite(cfg: SuiteConfig) -> Result<SuiteReport> {
    let mut suite = SuiteReport::new(cfg.server.clone());

    // Probe each listener up front so we can report which phases will
    // be skipped *before* the user waits through their durations.
    let availability = probe_listeners(&cfg).await;
    eprintln!(
        "{}",
        format!(
            "Suite probe: tcp={} udp={} http={} https={}",
            ok(availability.tcp),
            ok(availability.udp),
            ok(availability.http),
            ok(availability.https),
        )
        .bright_white()
        .bold()
    );

    let dur_secs = cfg.phase_duration.as_secs();

    // ── TCP ─────────────────────────────────────────────────────────
    if availability.tcp {
        run_phase(&mut suite, "tcp/latency", run_tcp_phase(&cfg, TestType::LatencyOnly, dur_secs)).await;
        run_phase(&mut suite, "tcp/bidirectional", run_tcp_phase(&cfg, TestType::Bidirectional, dur_secs)).await;
        run_phase(&mut suite, "tcp/full-duplex", run_tcp_phase(&cfg, TestType::FullDuplex, dur_secs)).await;
    } else {
        suite.skip("tcp/*", "TCP listener not reachable");
    }

    // ── UDP ─────────────────────────────────────────────────────────
    if availability.udp {
        run_phase(&mut suite, "udp/latency", run_udp_phase(&cfg, TestType::LatencyOnly, dur_secs)).await;
        run_phase(&mut suite, "udp/bidirectional", run_udp_phase(&cfg, TestType::Bidirectional, dur_secs)).await;
    } else {
        suite.skip("udp/*", "UDP listener not reachable");
    }

    // ── HTTP/1.1 + h2c (cleartext) ──────────────────────────────────
    if availability.http {
        run_phase(
            &mut suite,
            "http1/bidirectional",
            run_http_phase(&cfg, HttpVersion::HTTP1, cfg.http_port, dur_secs),
        )
        .await;
        run_phase(
            &mut suite,
            "h2c/bidirectional",
            run_http_phase(&cfg, HttpVersion::H2C, cfg.http_port, dur_secs),
        )
        .await;
    } else {
        suite.skip("http1/*", "HTTP listener not reachable");
        suite.skip("h2c/*", "HTTP listener not reachable");
    }

    // ── HTTPS (HTTP/2) ──────────────────────────────────────────────
    if cfg.include_tls && availability.https {
        run_phase(
            &mut suite,
            "http2/bidirectional",
            run_http_phase(&cfg, HttpVersion::HTTP2, cfg.https_port, dur_secs),
        )
        .await;
    } else if cfg.include_tls {
        suite.skip("http2/*", "HTTPS listener not reachable");
    }

    // HTTP/3 server is not yet implemented in this binary's `server`
    // command. The client supports it, so users can target an external
    // h3 server; we don't run it here automatically since we can't
    // probe it without a full TLS+QUIC handshake.
    suite.skip(
        "http3/*",
        "HTTP/3 server is not implemented in speed-cli yet (roadmap)",
    );

    suite.finalize();
    Ok(suite)
}

#[derive(Debug, Default, Clone, Copy)]
struct Availability {
    tcp: bool,
    udp: bool,
    http: bool,
    https: bool,
}

fn ok(b: bool) -> &'static str {
    if b { "✓" } else { "✗" }
}

async fn probe_listeners(cfg: &SuiteConfig) -> Availability {
    let mut a = Availability::default();
    let probe_to = Duration::from_secs(2);

    a.tcp = matches!(
        timeout(
            probe_to,
            TcpStream::connect(format!("{}:{}", cfg.server, cfg.tcp_udp_port)),
        )
        .await,
        Ok(Ok(_))
    );

    // For UDP we send a tiny PING and wait briefly for any response.
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await
        && sock
            .connect(format!("{}:{}", cfg.server, cfg.tcp_udp_port))
            .await
            .is_ok()
    {
        use crate::performance::udp::protocol::{BlasterPacket, now_us};
        let p = BlasterPacket::Ping {
            send_ts_us: now_us(),
        };
        if sock.send(&p.encode_to_vec(None)).await.is_ok() {
            let mut buf = [0u8; 256];
            a.udp = matches!(timeout(probe_to, sock.recv(&mut buf)).await, Ok(Ok(_)));
        }
    }

    // HTTP / HTTPS: just see if the TCP port accepts. The deeper
    // protocol pre-flight is done by the per-phase runner.
    a.http = matches!(
        timeout(
            probe_to,
            TcpStream::connect(format!("{}:{}", cfg.server, cfg.http_port)),
        )
        .await,
        Ok(Ok(_))
    );
    a.https = matches!(
        timeout(
            probe_to,
            TcpStream::connect(format!("{}:{}", cfg.server, cfg.https_port)),
        )
        .await,
        Ok(Ok(_))
    );

    a
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
    cfg: &SuiteConfig,
    test_type: TestType,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let payload_sizes: Vec<usize> = if matches!(test_type, TestType::LatencyOnly) {
        Vec::new() // unused
    } else {
        vec![65536]
    };
    let connections = if matches!(test_type, TestType::LatencyOnly) {
        1
    } else {
        cfg.connections
    };
    let conf = TcpTestConfig::new(
        cfg.server.clone(),
        Some(cfg.tcp_udp_port),
        duration,
        connections,
        test_type,
        payload_sizes,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting);
    run_tcp_client(conf).await
}

async fn run_udp_phase(
    cfg: &SuiteConfig,
    test_type: TestType,
    duration: u64,
) -> Result<crate::report::TestReport> {
    let payload_sizes: Vec<usize> = if matches!(test_type, TestType::LatencyOnly) {
        Vec::new()
    } else {
        vec![1200]
    };
    let conf = UdpTestConfig::new(
        cfg.server.clone(),
        Some(cfg.tcp_udp_port),
        duration,
        1,
        test_type,
        payload_sizes,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting)
    .with_target_rate_bps(cfg.udp_target_rate_mbps.saturating_mul(1_000_000));
    run_udp_client(conf).await
}

async fn run_http_phase(
    cfg: &SuiteConfig,
    version: HttpVersion,
    port: u16,
    duration: u64,
) -> Result<crate::report::TestReport> {
    // Smaller payload than the standalone HTTP test - we want this
    // suite to finish in minutes, not hours. 8 MB is large enough that
    // the test isn't dominated by request setup but small enough to
    // complete within the phase duration on a 100 Mbps link.
    let payload_sizes: Vec<usize> = vec![8 * 1024 * 1024];
    let conf = HttpTestConfig::new(
        cfg.server.clone(),
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
