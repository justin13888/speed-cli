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

use crate::control::{Handshake, TestTransport, perform_handshake};
use crate::performance::http::HttpVersion;
use crate::performance::http::client::run_http_test;
use crate::performance::quic::client::run_quic_client;
use crate::performance::tcp::client::run_tcp_client;
use crate::performance::udp::client::run_udp_client;
use crate::report::{
    HttpTestConfig, PhaseParams, QuicTestConfig, SuiteReport, TcpTestConfig, ThroughputAccounting,
    UdpTestConfig,
};
use crate::{CongestionAlgorithm, TestType};

/// Shared I/O unit for the suite: the TCP/QUIC per-operation throughput
/// payload *and* the HTTP chunk size. Unifying these is what makes the
/// per-protocol rows comparable.
const SUITE_IO_SIZE: usize = 64 * 1024;
/// Total HTTP request body. Larger than [`SUITE_IO_SIZE`] so HTTP phases
/// are not dominated by per-request setup cost; streamed in
/// `SUITE_IO_SIZE`-sized chunks.
const SUITE_HTTP_PAYLOAD: usize = 8 * 1024 * 1024;
/// UDP datagram size. An intrinsic exception to [`SUITE_IO_SIZE`]: it
/// must stay below the path MTU to avoid IP fragmentation, which would
/// make the UDP test measure something qualitatively different.
const SUITE_UDP_DATAGRAM: usize = 1200;
/// Sanity ceiling on the auto-derived parallel-stream count, so a high-core
/// client doesn't spawn pathologically many flows. A handful of streams is
/// enough to saturate almost any path; past that the marginal flow adds little
/// and only inflates client-side bookkeeping.
const MAX_AUTO_CONNECTIONS: usize = 8;

/// Default parallel-stream count when the user doesn't pass `--connections`.
///
/// Parallel streams exist to **saturate the network path** — the usual
/// throughput bottleneck — by overcoming per-flow limits (congestion control,
/// the bandwidth-delay product), not to exploit client/server compute. There
/// is no universal optimum (it depends on the link, RTT, and the server's
/// hardware, none of which the client can see), so we scale with the client's
/// available parallelism as a coarse proxy, capped at [`MAX_AUTO_CONNECTIONS`].
///
/// We deliberately use **logical** cores ([`std::thread::available_parallelism`],
/// which counts SMT siblings): the streams are async I/O-bound tasks, not
/// CPU-pinned threads, so the extra flows cost essentially nothing on the CPU,
/// and we'd rather over- than under-provision flows and under-measure the link.
/// Falls back to 4 if the platform won't report a core count.
pub fn default_connections() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, MAX_AUTO_CONNECTIONS)
}

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
    /// Shared I/O unit: TCP/QUIC throughput payload and HTTP chunk size.
    pub io_size: usize,
    /// Total HTTP request body size, streamed in `io_size` chunks.
    pub http_payload: usize,
    pub accounting: ThroughputAccounting,
    /// When false, TLS phases (HTTP/2-TLS, HTTP/3) are force-skipped
    /// even if the server advertises them.
    pub include_tls: bool,
    /// Congestion controller for the raw-QUIC and HTTP/3 phases. A
    /// single knob, deliberately not a matrix dimension: the phase
    /// count is identical for cubic and bbr. TCP-based phases always
    /// use the OS controller.
    pub congestion: CongestionAlgorithm,
}

impl SuiteConfig {
    pub fn new(server: String) -> Self {
        Self {
            server,
            control_port: crate::constants::DEFAULT_CONTROL_PORT,
            phase_duration: Duration::from_secs(8),
            warmup: Duration::from_secs(1),
            connections: default_connections(),
            udp_target_rate_mbps: 100,
            io_size: SUITE_IO_SIZE,
            http_payload: SUITE_HTTP_PAYLOAD,
            accounting: ThroughputAccounting::Goodput,
            include_tls: true,
            congestion: CongestionAlgorithm::default(),
        }
    }
}

/// Drive the whole suite. The handshake runs first: a protocol-version
/// mismatch aborts the entire suite immediately (no point running
/// phases against an incompatible server).
pub async fn run_suite(cfg: SuiteConfig) -> Result<SuiteReport> {
    let handshake = perform_handshake(&cfg.server, cfg.control_port).await?;
    let mut suite = SuiteReport::new(cfg.server.clone());

    // ── TCP ─────────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::TcpRaw).is_some() {
        for tt in [
            TestType::LatencyOnly,
            TestType::Bidirectional,
            TestType::FullDuplex,
        ] {
            let params = tcp_quic_params(&cfg, tt);
            let label = format!("tcp/{}", phase_suffix(tt));
            run_phase(
                &mut suite,
                &label,
                params.clone(),
                run_tcp_phase(&handshake, &cfg, params),
            )
            .await;
        }
    } else {
        suite.skip("tcp/*", "TCP listener not advertised by server");
    }

    // ── UDP ─────────────────────────────────────────────────────────
    // No full-duplex row: UDP has no single-socket bidirectional mode.
    if handshake
        .manifest
        .listener(TestTransport::UdpBlaster)
        .is_some()
    {
        for tt in [TestType::LatencyOnly, TestType::Bidirectional] {
            let params = udp_params(&cfg, tt);
            let label = format!("udp/{}", phase_suffix(tt));
            run_phase(
                &mut suite,
                &label,
                params.clone(),
                run_udp_phase(&handshake, &cfg, params),
            )
            .await;
        }
    } else {
        suite.skip("udp/*", "UDP listener not advertised by server");
    }

    // ── Raw QUIC ────────────────────────────────────────────────────
    if handshake
        .manifest
        .listener(TestTransport::QuicRaw)
        .is_some()
    {
        for tt in [
            TestType::LatencyOnly,
            TestType::Bidirectional,
            TestType::FullDuplex,
        ] {
            let params = tcp_quic_params(&cfg, tt);
            let label = format!("quic/{}", phase_suffix(tt));
            run_phase(
                &mut suite,
                &label,
                params.clone(),
                run_quic_phase(&handshake, &cfg, params),
            )
            .await;
        }
    } else {
        suite.skip("quic/*", "raw-QUIC listener not advertised by server");
    }

    // ── HTTP/1.1 ────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::Http1).is_some() {
        run_http_set(
            &mut suite,
            &handshake,
            &cfg,
            "http1",
            HttpVersion::HTTP1,
            TestTransport::Http1,
        )
        .await;
    } else {
        suite.skip("http1/*", "HTTP/1.1 listener not advertised by server");
    }

    // ── h2c ─────────────────────────────────────────────────────────
    if handshake.manifest.listener(TestTransport::H2c).is_some() {
        run_http_set(
            &mut suite,
            &handshake,
            &cfg,
            "h2c",
            HttpVersion::H2C,
            TestTransport::H2c,
        )
        .await;
    } else {
        suite.skip("h2c/*", "h2c listener not advertised by server");
    }

    // ── HTTP/2 over TLS ─────────────────────────────────────────────
    if !cfg.include_tls {
        suite.skip("http2/*", "TLS phases skipped (--no-tls)");
    } else if handshake
        .manifest
        .listener(TestTransport::Http2Tls)
        .is_some()
    {
        run_http_set(
            &mut suite,
            &handshake,
            &cfg,
            "http2",
            HttpVersion::HTTP2,
            TestTransport::Http2Tls,
        )
        .await;
    } else {
        suite.skip("http2/*", "HTTP/2-TLS listener not advertised by server");
    }

    // ── HTTP/3 ──────────────────────────────────────────────────────
    if !cfg.include_tls {
        suite.skip("http3/*", "TLS phases skipped (--no-tls)");
    } else if handshake.manifest.listener(TestTransport::Http3).is_some() {
        run_http_set(
            &mut suite,
            &handshake,
            &cfg,
            "http3",
            HttpVersion::HTTP3,
            TestTransport::Http3,
        )
        .await;
    } else {
        suite.skip("http3/*", "HTTP/3 listener not advertised by server");
    }

    suite.finalize();
    Ok(suite)
}

/// Phase-label suffix for a test type. Kept stable and protocol-agnostic
/// so the suite matrix lines up row-for-row: the HTTP `*/full-duplex`
/// rows run [`TestType::Simultaneous`] but still report under the
/// `full-duplex` suffix.
fn phase_suffix(tt: TestType) -> &'static str {
    match tt {
        TestType::LatencyOnly => "latency",
        TestType::LatencyUnderLoad => "latency-under-load",
        TestType::Bidirectional => "bidirectional",
        TestType::FullDuplex | TestType::Simultaneous => "full-duplex",
        TestType::Download => "download",
        TestType::Upload => "upload",
    }
}

/// Effective parameters for a TCP or raw-QUIC phase. Both share the
/// suite I/O size and the same connection convention (1 for latency).
fn tcp_quic_params(cfg: &SuiteConfig, test_type: TestType) -> PhaseParams {
    let is_latency = matches!(test_type, TestType::LatencyOnly);
    PhaseParams {
        payload_size: (!is_latency).then_some(cfg.io_size),
        io_unit: cfg.io_size,
        connections: if is_latency { 1 } else { cfg.connections },
        duration: cfg.phase_duration,
        test_type,
        deviations: Vec::new(),
    }
}

/// Effective parameters for a UDP phase. UDP uses the same parallel-stream
/// count as the other protocols (each stream is its own socket / server
/// session); its one intrinsic difference is the MTU-bound datagram size,
/// recorded as a deviation.
fn udp_params(cfg: &SuiteConfig, test_type: TestType) -> PhaseParams {
    let is_latency = matches!(test_type, TestType::LatencyOnly);
    PhaseParams {
        payload_size: (!is_latency).then_some(SUITE_UDP_DATAGRAM),
        io_unit: SUITE_UDP_DATAGRAM,
        connections: if is_latency { 1 } else { cfg.connections },
        duration: cfg.phase_duration,
        test_type,
        deviations: vec![
            "UDP datagram is MTU-bound (1200 B) by design; it intentionally does not use the \
             64 KB TCP/QUIC I/O unit, to avoid IP fragmentation"
                .to_string(),
        ],
    }
}

/// Effective parameters for an HTTP phase of any version. The HTTP
/// chunk size is the suite I/O unit; the bulk request body is the
/// larger `http_payload`.
fn http_params(cfg: &SuiteConfig, test_type: TestType) -> PhaseParams {
    let is_latency = matches!(test_type, TestType::LatencyOnly);
    let mut deviations = Vec::new();
    if matches!(test_type, TestType::Simultaneous) {
        deviations.push(
            "HTTP is request/response; the full-duplex row runs Simultaneous \
             (parallel up/down streams) as the closest analog"
                .to_string(),
        );
    }
    PhaseParams {
        payload_size: (!is_latency).then_some(cfg.http_payload),
        io_unit: cfg.io_size,
        connections: if is_latency { 1 } else { cfg.connections },
        duration: cfg.phase_duration,
        test_type,
        deviations,
    }
}

async fn run_phase<F>(suite: &mut SuiteReport, label: &str, params: PhaseParams, fut: F)
where
    F: std::future::Future<Output = Result<crate::report::TestReport>>,
{
    tracing::info!(
        "{}",
        format!("\n── Suite phase: {label} ──")
            .bright_magenta()
            .bold()
    );
    match fut.await {
        Ok(report) => suite.record(label, params, report),
        Err(e) => {
            tracing::warn!(phase = label, error = %e, "suite phase failed; continuing");
            suite.skip(label, e.to_string());
        }
    }
}

/// Run the full HTTP matrix (latency / bidirectional / full-duplex) for
/// one HTTP version against one advertised listener.
async fn run_http_set(
    suite: &mut SuiteReport,
    handshake: &Handshake,
    cfg: &SuiteConfig,
    proto: &str,
    version: HttpVersion,
    transport: TestTransport,
) {
    for tt in [
        TestType::LatencyOnly,
        TestType::Bidirectional,
        TestType::Simultaneous,
    ] {
        let params = http_params(cfg, tt);
        let label = format!("{proto}/{}", phase_suffix(tt));
        run_phase(
            suite,
            &label,
            params.clone(),
            run_http_phase(handshake, cfg, version, transport, params),
        )
        .await;
    }
}

async fn run_tcp_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    params: PhaseParams,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(TestTransport::TcpRaw)?;
    let payload_sizes: Vec<usize> = params.payload_size.into_iter().collect();
    let conf = TcpTestConfig::new(
        host,
        Some(port),
        params.duration.as_secs(),
        params.connections,
        params.test_type,
        payload_sizes,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting);
    run_tcp_client(conf).await
}

async fn run_udp_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    params: PhaseParams,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint(TestTransport::UdpBlaster)?;
    let payload_sizes: Vec<usize> = params.payload_size.into_iter().collect();
    let conf = UdpTestConfig::new(
        host,
        Some(port),
        params.duration.as_secs(),
        params.connections,
        params.test_type,
        payload_sizes,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting)
    .with_target_rate_bps(cfg.udp_target_rate_mbps.saturating_mul(1_000_000));
    run_udp_client(conf).await
}

async fn run_quic_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    params: PhaseParams,
) -> Result<crate::report::TestReport> {
    let (host, port) = handshake.endpoint_with(TestTransport::QuicRaw, cfg.congestion)?;
    let payload_sizes: Vec<usize> = params.payload_size.into_iter().collect();
    let conf = QuicTestConfig::new(
        host,
        Some(port),
        params.duration.as_secs(),
        params.connections,
        params.test_type,
        payload_sizes,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting)
    .with_congestion(cfg.congestion);
    run_quic_client(conf).await
}

async fn run_http_phase(
    handshake: &Handshake,
    cfg: &SuiteConfig,
    version: HttpVersion,
    transport: TestTransport,
    params: PhaseParams,
) -> Result<crate::report::TestReport> {
    // Only HTTP/3 rides QUIC; the TCP-backed versions have no
    // congestion listener variants to choose between.
    let (host, port) = if transport == TestTransport::Http3 {
        handshake.endpoint_with(transport, cfg.congestion)?
    } else {
        handshake.endpoint(transport)?
    };
    // For latency phases `payload_size` is `None`; `HttpTestConfig::new`
    // backfills DEFAULT_HTTP_PAYLOAD_SIZES on an empty set, so pass the
    // I/O unit explicitly to keep the recorded config honest. The HTTP
    // chunk size is unified to the suite I/O unit.
    let payload_sizes: Vec<usize> = vec![params.payload_size.unwrap_or(cfg.io_size)];
    let conf = HttpTestConfig::new(
        host,
        Some(port),
        params.duration.as_secs(),
        params.connections,
        params.test_type,
        payload_sizes,
        Some(cfg.io_size),
        version,
    )
    .with_warmup(cfg.warmup)
    .with_accounting(cfg.accounting)
    .with_congestion(if transport == TestTransport::Http3 {
        cfg.congestion
    } else {
        CongestionAlgorithm::default()
    });
    run_http_test(conf).await
}
