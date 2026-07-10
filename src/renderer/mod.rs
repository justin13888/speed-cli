use crate::build_info::BuildInfo;
use crate::performance::http::HttpVersion;
use crate::report::*;
use crate::utils::env::Environment;
use crate::utils::types::TestType;
use indexmap::IndexMap;
use std::io::{self, Write};

mod graph;
use graph::latency_svg;

/// Build provenance for the report header's "Version" cell: a one-line
/// summary plus `<br>`-separated commit / rustc / build-time detail.
/// Values are build-controlled (never user input), so no escaping.
fn build_meta_html(build: &BuildInfo) -> String {
    let commit = match &build.git_commit {
        Some(c) => {
            let dirty = match build.git_dirty {
                Some(true) => " (dirty)",
                Some(false) => "",
                None => " (dirty: unknown)",
            };
            format!("{c}{dirty}")
        }
        None => "unknown".to_string(),
    };
    format!(
        "{summary}<br>commit: {commit}<br>rustc: {rustc}<br>built: {built}",
        summary = build.summary(),
        rustc = build.rustc,
        built = build.build_timestamp,
    )
}

/// Shared stylesheet for every HTML report. Injected exactly once per
/// document by [`write_document_start`], so embedding a `TestReport`'s
/// sections inside a suite page never duplicates it.
const REPORT_CSS: &str = r#"
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            line-height: 1.6;
            margin: 0;
            padding: 20px;
            background-color: #f5f5f5;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
            background-color: white;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
            padding: 30px;
        }
        .header {
            text-align: center;
            border-bottom: 3px solid #007acc;
            padding-bottom: 20px;
            margin-bottom: 30px;
        }
        .header h1 {
            color: #007acc;
            margin: 0;
            font-size: 2.5em;
        }
        .meta-info {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 15px;
            margin-bottom: 30px;
            padding: 20px;
            background-color: #f8f9fa;
            border-radius: 6px;
        }
        .meta-item {
            display: flex;
            justify-content: space-between;
        }
        .meta-label {
            font-weight: 600;
            color: #495057;
        }
        .meta-value {
            color: #007acc;
            font-weight: 500;
        }
        .section {
            margin-bottom: 30px;
        }
        .section-title {
            color: #495057;
            border-bottom: 2px solid #e9ecef;
            padding-bottom: 10px;
            margin-bottom: 20px;
            font-size: 1.5em;
            font-weight: 600;
        }
        .config-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
        }
        .config-card {
            background-color: #f8f9fa;
            padding: 20px;
            border-radius: 6px;
            border-left: 4px solid #007acc;
        }
        .phase-section {
            margin-top: 40px;
            border-top: 3px solid #e9ecef;
            padding-top: 20px;
        }
        .params-card {
            background-color: #f8f9fa;
            padding: 12px 20px;
            border-radius: 6px;
            border-left: 4px solid #6f42c1;
            margin-bottom: 20px;
        }
        .overview-table {
            width: 100%;
            border-collapse: collapse;
        }
        .overview-table th, .overview-table td {
            padding: 8px 12px;
            border-bottom: 1px solid #e9ecef;
            text-align: left;
        }
        .overview-table th {
            color: #495057;
        }
        .overview-table a {
            color: #007acc;
            text-decoration: none;
        }
        .skip-table {
            width: 100%;
            border-collapse: collapse;
        }
        .skip-table td {
            padding: 8px 12px;
            border-bottom: 1px solid #e9ecef;
        }
        .banner {
            padding: 20px;
            background-color: #fff3cd;
            border-radius: 6px;
            color: #856404;
        }
"#;

/// Open an HTML document: doctype, head with [`REPORT_CSS`], and the
/// page container. Pair every call with [`write_document_end`].
fn write_document_start<W: Write>(writer: &mut W, title: &str) -> io::Result<()> {
    write!(
        writer,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <style>{REPORT_CSS}    </style>
</head>
<body>
    <div class="container">
"#
    )
}

fn write_document_end<W: Write>(writer: &mut W) -> io::Result<()> {
    write!(
        writer,
        r#"
    </div>
</body>
</html>"#
    )
}

/// Minimal HTML escaper for user- or remote-derived strings (phase
/// labels, hostnames, skip reasons, deviation notes). Build- and
/// config-controlled values skip this.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Tail percentiles, spike verdict, and the time-vs-latency SVG chart, rendered
/// below a `LatencyResult`'s numeric grid. `overlay`, when present, is drawn as
/// a dashed reference series on the chart (used for the idle-vs-loaded view).
fn latency_extras_html(result: &LatencyResult, overlay: Option<&LatencyResult>) -> String {
    let mut s = String::new();

    if let (Some(p95), Some(p99)) = (result.p95_rtt(), result.p99_rtt()) {
        let p999 = result.p999_rtt().unwrap_or(p99);
        s.push_str(&format!(
            r#"<div style="display: flex; justify-content: space-between; margin-top: 10px;">
                <strong>Tail RTT (p95 / p99 / p99.9):</strong>
                <span style="color: #fd7e14;">{p95:.2} / {p99:.2} / {p999:.2} ms</span>
            </div>"#
        ));
    }

    if let Some(sr) = result.spike_report() {
        let color = match sr.verdict {
            SpikeVerdict::Clean => "#28a745",
            SpikeVerdict::Occasional => "#fd7e14",
            SpikeVerdict::Frequent => "#dc3545",
        };
        s.push_str(&format!(
            r#"<div style="margin-top: 8px; color: {color};"><strong>Spikes:</strong> {sr}</div>"#
        ));
    }

    s.push_str(&latency_svg(result, overlay));
    s
}

/// The "Latency Under Load" section for a `NetworkTestResult`: a bufferbloat
/// headline, an idle-vs-loaded comparison chart, and the loaded numeric detail.
/// Empty when no under-load series was captured.
/// HTTP-only "requests/sec" row appended after a throughput result block.
/// Empty for other protocols, where a sample is a read/packet, not a request.
fn rps_html(result: &ThroughputResult, protocol: crate::report::NetworkProtocol) -> String {
    if matches!(protocol, crate::report::NetworkProtocol::Http) {
        format!(
            r#"<div style="margin-top:6px;"><span style="color:#007acc;">Requests/sec:</span> {:.1}</div>"#,
            result.requests_per_second()
        )
    } else {
        String::new()
    }
}

/// Connection-establishment timings (handshake / TTFB) section, or empty
/// when none were captured for this protocol.
fn connection_html(result: &NetworkTestResult, prefix: &str) -> String {
    let Some(conn) = &result.connection else {
        return String::new();
    };
    let ms = |us: u64| format!("{:.2} ms", us as f64 / 1000.0);
    let mut rows = String::new();
    let mut row = |label: &str, us: Option<u64>| {
        if let Some(us) = us {
            rows.push_str(&format!(
                r#"<div><span style="color:#007acc;">{label}:</span> {}</div>"#,
                ms(us)
            ));
        }
    };
    row("TCP handshake", conn.tcp_handshake_us);
    row("QUIC handshake (incl. TLS)", conn.quic_handshake_us);
    row("Time to first byte", conn.ttfb_us);
    if rows.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="result-section" style="margin-bottom: 30px;">
            <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{prefix}Connection</h3>
            <div style="margin-left: 20px;">{rows}</div>
        </div>"#
    )
}

fn under_load_html(result: &NetworkTestResult, prefix: &str) -> String {
    let Some(loaded) = &result.latency_under_load else {
        return String::new();
    };
    let mut s = format!(
        r#"<div class="result-section" style="margin-bottom: 30px;">
            <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{prefix}Latency Under Load</h3>"#
    );

    if let Some(inf) = result.bufferbloat_inflation() {
        let color = if inf.is_severe() {
            "#dc3545"
        } else if inf.is_mild() {
            "#fd7e14"
        } else {
            "#28a745"
        };
        s.push_str(&format!(
            r#"<div style="margin: 8px 0; color: {color}; font-weight: 600;">
                Bufferbloat: median {dm:+.1} ms, p99 {dp:+.1} ms under load
                (idle {i50:.1}/{i99:.1} &rarr; loaded {l50:.1}/{l99:.1} ms)
            </div>"#,
            dm = inf.d_median_ms(),
            dp = inf.d_p99_ms(),
            i50 = inf.idle_p50,
            i99 = inf.idle_p99,
            l50 = inf.loaded_p50,
            l99 = inf.loaded_p99,
        ));
    }

    // Idle-vs-loaded comparison chart, then the loaded numeric detail.
    if let Some(idle) = &result.latency {
        s.push_str(&latency_svg(loaded, Some(idle)));
    }
    s.push_str(&loaded.to_html());
    s.push_str("</div>");
    s
}

/// Trait for converting structs/enums related to `TestReport` into HTML representation.
///
/// This trait supports both streaming writes to any `Write` implementation and
/// collecting the output as a `String`. This allows for memory-efficient processing
/// of large reports while maintaining backwards compatibility.
///
/// This trait is implemented for all major types in the speed-cli reporting system:
/// - `TestReport` - The main test report structure
/// - `SuiteReport` - The composite suite report (embeds each phase's `TestReport`)
/// - `TestConfig` and its variants (`TcpTestConfig`, `UdpTestConfig`, `HttpTestConfig`)
/// - `TestResult` and its variants (`ThroughputResult`, `NetworkTestResult`)
/// - `LatencyResult` and `LatencyMeasurement`
/// - `ThroughputMeasurement`
/// - Enum types like `TestType` and `HttpVersion`
///
/// ## Example
///
/// ```ignore
/// use speed_cli::renderer::ToHtml;
/// use speed_cli::report::TestReport;
/// use std::fs::File;
/// use std::io::BufWriter;
///
/// // Stream directly to file (memory efficient for large reports)
/// let file = File::create("report.html")?;
/// let mut writer = BufWriter::new(file);
/// report.write_html(&mut writer)?;
///
/// // Or collect as string (convenient for small reports)
/// let html = report.to_html();
/// std::fs::write("report.html", html)?;
/// ```
pub trait ToHtml {
    /// Write the HTML representation to a `Write` implementation
    ///
    /// This method streams the HTML output, making it memory-efficient
    /// for large reports.
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()>;

    /// Convert the object to HTML string
    ///
    /// This is a convenience method that collects all output into a `String`.
    /// For large reports, prefer `write_html` for better memory efficiency.
    fn to_html(&self) -> String {
        let mut buffer = Vec::new();
        self.write_html(&mut buffer)
            .expect("Writing to Vec should never fail");
        String::from_utf8(buffer).expect("HTML output should be valid UTF-8")
    }
}

// Implementation for TestReport
impl ToHtml for TestReport {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_document_start(writer, "Speed CLI Test Report")?;
        write!(
            writer,
            r#"        <div class="header">
            <h1>═══ Speed CLI Test Report ═══</h1>
        </div>

        <div class="meta-info">
            <div class="meta-item">
                <span class="meta-label">Version:</span>
                <span class="meta-value">{}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Start Time:</span>
                <span class="meta-value">{}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Report Time:</span>
                <span class="meta-value">{}</span>
            </div>
        </div>
"#,
            build_meta_html(&self.build),
            self.start_time.format("%Y-%m-%d %H:%M:%S UTC"),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;

        write_report_sections(self, writer)?;
        write_document_end(writer)
    }
}

/// The Configuration and Results sections of a single [`TestReport`],
/// without the document shell. Shared by the standalone report page and
/// each phase of a suite page.
pub(crate) fn write_report_sections<W: Write>(
    report: &TestReport,
    writer: &mut W,
) -> io::Result<()> {
    write!(
        writer,
        r#"
        <div class="section">
            <h2 class="section-title">Configuration</h2>
            <div class="config-grid">
                <div class="config-card">
                    "#
    )?;

    report.config.write_html(writer)?;

    write!(
        writer,
        r#"
                </div>
            </div>
        </div>

        <div class="section">
            <h2 class="section-title">Results</h2>
            "#
    )?;

    report.result.write_html(writer)?;

    write!(writer, "\n        </div>")
}

// Implementation for TestConfig
impl ToHtml for TestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            TestConfig::Tcp(config) => config.write_html(writer),
            TestConfig::Udp(config) => config.write_html(writer),
            TestConfig::Http(config) => config.write_html(writer),
            TestConfig::Quic(config) => config.write_html(writer),
        }
    }

    fn to_html(&self) -> String {
        match self {
            TestConfig::Tcp(config) => config.to_html(),
            TestConfig::Udp(config) => config.to_html(),
            TestConfig::Http(config) => config.to_html(),
            TestConfig::Quic(config) => config.to_html(),
        }
    }
}

// Implementation for QuicTestConfig
impl ToHtml for QuicTestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            r#"<h3 style="color: #28a745; margin-top: 0;">QUIC Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">QUIC</span></div>
                <div><strong>Server:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Port:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Streams:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server,
            self.port,
            self.duration.as_secs(),
            self.parallel_connections,
            self.test_type.to_html(),
            payload_sizes
        )
    }
}

// Implementation for TcpTestConfig
impl ToHtml for TcpTestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        write!(
            writer,
            r#"<h3 style="color: #28a745; margin-top: 0;">TCP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">TCP</span></div>
                <div><strong>Server:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Port:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Connections:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server,
            self.port,
            self.duration.as_secs(),
            self.parallel_connections,
            self.test_type.to_html(),
            payload_sizes
        )
    }

    fn to_html(&self) -> String {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            r#"<h3 style="color: #28a745; margin-top: 0;">TCP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">TCP</span></div>
                <div><strong>Server:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Port:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Connections:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server,
            self.port,
            self.duration.as_secs(),
            self.parallel_connections,
            self.test_type.to_html(),
            payload_sizes
        )
    }
}

// Implementation for UdpTestConfig
impl ToHtml for UdpTestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        write!(
            writer,
            r#"<h3 style="color: #28a745; margin-top: 0;">UDP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">UDP</span></div>
                <div><strong>Server:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Port:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Streams:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server,
            self.port,
            self.duration,
            self.parallel_streams,
            self.test_type.to_html(),
            payload_sizes
        )
    }

    fn to_html(&self) -> String {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            r#"<h3 style="color: #28a745; margin-top: 0;">UDP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">UDP</span></div>
                <div><strong>Server:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Port:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Streams:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server,
            self.port,
            self.duration,
            self.parallel_streams,
            self.test_type.to_html(),
            payload_sizes
        )
    }
}

// Implementation for HttpTestConfig
impl ToHtml for HttpTestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        write!(
            writer,
            r#"<h3 style="color: #28a745; margin-top: 0;">HTTP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">HTTP</span></div>
                <div><strong>Server URL:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Connections:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>HTTP Version:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server_url,
            self.duration.as_secs(),
            self.parallel_connections,
            self.test_type.to_html(),
            self.http_version.to_html(),
            payload_sizes
        )
    }

    fn to_html(&self) -> String {
        let payload_sizes = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes_usize(*s))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            r#"<h3 style="color: #28a745; margin-top: 0;">HTTP Configuration</h3>
            <div style="display: grid; gap: 10px;">
                <div><strong>Protocol:</strong> <span style="color: #28a745;">HTTP</span></div>
                <div><strong>Server URL:</strong> <span style="color: #007acc;">{}</span></div>
                <div><strong>Duration:</strong> <span style="color: #6f42c1;">{}s</span></div>
                <div><strong>Parallel Connections:</strong> <span style="color: #28a745;">{}</span></div>
                <div><strong>Test Type:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>HTTP Version:</strong> <span style="color: #fd7e14;">{}</span></div>
                <div><strong>Payload Sizes:</strong> <span style="color: #6c757d;">[{}]</span></div>
            </div>"#,
            self.server_url,
            self.duration.as_secs(),
            self.parallel_connections,
            self.test_type.to_html(),
            self.http_version.to_html(),
            payload_sizes
        )
    }
}

// Implementation for TestResult
impl ToHtml for TestResult {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            TestResult::Simple(result) => result.write_html(writer),
            TestResult::Network(result) => result.write_html(writer),
        }
    }

    fn to_html(&self) -> String {
        match self {
            TestResult::Simple(result) => result.to_html(),
            TestResult::Network(result) => result.to_html(),
        }
    }
}

// Implementation for ThroughputResult
impl ToHtml for ThroughputResult {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(
            writer,
            r#"<div class="result-card" style="background-color: #f8f9fa; padding: 20px; border-radius: 6px; border-left: 4px solid #28a745;">
                <h3 style="color: #28a745; margin-top: 0;">Throughput Results</h3>
                <div style="display: grid; gap: 15px;">
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Data Transferred:</strong> 
                        <span style="color: #007acc;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Duration:</strong> 
                        <span style="color: #fd7e14;">{:.2}s</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Average Throughput:</strong> 
                        <span style="color: #6f42c1;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Measurements:</strong> 
                        <span style="color: #6c757d;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Timestamp:</strong> 
                        <span style="color: #007acc;">{}</span>
                    </div>
                </div>
            </div>"#,
            format_bytes_u64(self.bytes_transferred()),
            self.total_duration().as_secs_f64(),
            // avg_throughput() is bytes/sec; format_throughput expects bits/sec.
            format_throughput(self.avg_throughput() * 8.0),
            self.sample_count(),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )
    }

    fn to_html(&self) -> String {
        format!(
            r#"<div class="result-card" style="background-color: #f8f9fa; padding: 20px; border-radius: 6px; border-left: 4px solid #28a745;">
                <h3 style="color: #28a745; margin-top: 0;">Throughput Results</h3>
                <div style="display: grid; gap: 15px;">
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Data Transferred:</strong> 
                        <span style="color: #007acc;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Duration:</strong> 
                        <span style="color: #fd7e14;">{:.2}s</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Average Throughput:</strong> 
                        <span style="color: #6f42c1;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Measurements:</strong> 
                        <span style="color: #6c757d;">{}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Timestamp:</strong> 
                        <span style="color: #007acc;">{}</span>
                    </div>
                </div>
            </div>"#,
            format_bytes_u64(self.bytes_transferred()),
            self.total_duration().as_secs_f64(),
            // avg_throughput() is bytes/sec; format_throughput expects bits/sec.
            format_throughput(self.avg_throughput() * 8.0),
            self.sample_count(),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )
    }
}

// Implementation for NetworkTestResult
impl ToHtml for NetworkTestResult {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let protocol_prefix = match self.protocol {
            crate::report::NetworkProtocol::Http => "",
            crate::report::NetworkProtocol::Tcp => "TCP ",
            crate::report::NetworkProtocol::Udp => "UDP ",
            crate::report::NetworkProtocol::Quic => "QUIC ",
        };

        // Connection-establishment timings (handshake / TTFB).
        write!(writer, "{}", connection_html(self, protocol_prefix))?;

        // Latency results
        if let Some(latency) = &self.latency {
            write!(
                writer,
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Latency Results</h3>
                    "#,
                protocol_prefix
            )?;
            latency.write_html(writer)?;
            write!(writer, r#"</div>"#)?;
        }

        // Latency under load (WiFi / bufferbloat stress test).
        write!(writer, "{}", under_load_html(self, protocol_prefix))?;

        // Download results
        if !self.download.is_empty() {
            write!(
                writer,
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Download Results</h3>
                    <div style="display: grid; gap: 20px;">"#,
                protocol_prefix
            )?;
            for (size, result) in &self.download {
                write!(
                    writer,
                    r#"<div>
                        <h4 style="color: #007acc; margin-bottom: 10px;">Payload Size: {}</h4>
                        <div style="margin-left: 20px;">"#,
                    format_bytes_usize(*size)
                )?;
                result.write_html(writer)?;
                write!(writer, "{}", rps_html(result, self.protocol))?;
                write!(writer, r#"</div></div>"#)?;
            }
            write!(writer, r#"</div></div>"#)?;
        }

        // Upload results
        if !self.upload.is_empty() {
            write!(
                writer,
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Upload Results</h3>
                    <div style="display: grid; gap: 20px;">"#,
                protocol_prefix
            )?;
            for (size, result) in &self.upload {
                write!(
                    writer,
                    r#"<div>
                        <h4 style="color: #007acc; margin-bottom: 10px;">Payload Size: {}</h4>
                        <div style="margin-left: 20px;">"#,
                    format_bytes_usize(*size)
                )?;
                result.write_html(writer)?;
                write!(writer, "{}", rps_html(result, self.protocol))?;
                write!(writer, r#"</div></div>"#)?;
            }
            write!(writer, r#"</div></div>"#)?;
        }

        Ok(())
    }

    fn to_html(&self) -> String {
        let mut html = String::new();
        let protocol_prefix = match self.protocol {
            crate::report::NetworkProtocol::Http => "",
            crate::report::NetworkProtocol::Tcp => "TCP ",
            crate::report::NetworkProtocol::Udp => "UDP ",
            crate::report::NetworkProtocol::Quic => "QUIC ",
        };

        // Connection-establishment timings (handshake / TTFB).
        html.push_str(&connection_html(self, protocol_prefix));

        // Latency results
        if let Some(latency) = &self.latency {
            html.push_str(&format!(
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Latency Results</h3>
                    {}
                </div>"#,
                protocol_prefix,
                latency.to_html()
            ));
        }

        // Latency under load (WiFi / bufferbloat stress test).
        html.push_str(&under_load_html(self, protocol_prefix));

        // Download results
        if !self.download.is_empty() {
            html.push_str(&format!(
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Download Results</h3>
                    <div style="display: grid; gap: 20px;">{}</div>
                </div>"#,
                protocol_prefix,
                self.download
                    .iter()
                    .map(|(size, result)| format!(
                        r#"<div>
                            <h4 style="color: #007acc; margin-bottom: 10px;">Payload Size: {}</h4>
                            <div style="margin-left: 20px;">{}{}</div>
                        </div>"#,
                        format_bytes_usize(*size),
                        result.to_html(),
                        rps_html(result, self.protocol)
                    ))
                    .collect::<Vec<_>>()
                    .join("")
            ));
        }

        // Upload results
        if !self.upload.is_empty() {
            html.push_str(&format!(
                r#"<div class="result-section" style="margin-bottom: 30px;">
                    <h3 style="color: #28a745; border-bottom: 2px solid #e9ecef; padding-bottom: 10px;">{}Upload Results</h3>
                    <div style="display: grid; gap: 20px;">{}</div>
                </div>"#,
                protocol_prefix,
                self.upload
                    .iter()
                    .map(|(size, result)| format!(
                        r#"<div>
                            <h4 style="color: #007acc; margin-bottom: 10px;">Payload Size: {}</h4>
                            <div style="margin-left: 20px;">{}{}</div>
                        </div>"#,
                        format_bytes_usize(*size),
                        result.to_html(),
                        rps_html(result, self.protocol)
                    ))
                    .collect::<Vec<_>>()
                    .join("")
            ));
        }

        html
    }
}

// Implementation for LatencyResult
impl ToHtml for LatencyResult {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let total_count = self.count();
        let successful_count = self.successful_count();
        let dropped_count = self.dropped_count();
        let loss_rate = if total_count > 0 {
            (dropped_count as f64 / total_count as f64) * 100.0
        } else {
            0.0
        };

        write!(
            writer,
            r#"<div style="background-color: #f8f9fa; padding: 20px; border-radius: 6px; border-left: 4px solid #007acc;">
                <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 15px; margin-bottom: 20px;">
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Total Measurements:</strong> 
                        <span style="color: #6c757d;">{total_count}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Successful:</strong> 
                        <span style="color: #28a745;">{successful_count}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Dropped:</strong> 
                        <span style="color: #dc3545;">{dropped_count}</span>
                    </div>
                    <div style="display: flex; justify-content: space-between;">
                        <strong>Packet Loss:</strong> 
                        <span style="color: #dc3545;">{loss_rate:.2}%</span>
                    </div>"#
        )?;

        // Add RTT statistics if we have successful measurements
        if successful_count > 0 {
            if let Some(avg) = self.avg_rtt() {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Average RTT:</strong> 
                        <span style="color: #007acc;">{avg:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(min) = self.min_rtt() {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Min RTT:</strong> 
                        <span style="color: #28a745;">{min:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(p25) = self.percentile_rtt(25.0) {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>25th Percentile RTT:</strong> 
                        <span style="color: #fd7e14;">{p25:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(p50) = self.percentile_rtt(50.0) {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Median RTT:</strong> 
                        <span style="color: #fd7e14;">{p50:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(p75) = self.percentile_rtt(75.0) {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>75th Percentile RTT:</strong> 
                        <span style="color: #fd7e14;">{p75:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(max) = self.max_rtt() {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Max RTT:</strong> 
                        <span style="color: #fd7e14;">{max:.2} ms</span>
                    </div>"#
                )?;
            }

            if let Some(jitter) = self.rtt_stddev() {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Jitter (Std Dev):</strong> 
                        <span style="color: #6f42c1;">{jitter:.2} ms</span>
                    </div>"#
                )?;
            }
        }

        let extras = latency_extras_html(self, None);
        write!(
            writer,
            r#"</div>
                {extras}
                <div style="margin-top: 15px;">
                    <strong>Timestamp:</strong>
                    <span style="color: #007acc;">{}</span>
                </div>
            </div>"#,
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )
    }

    fn to_html(&self) -> String {
        let total_count = self.count();
        let successful_count = self.successful_count();
        let dropped_count = self.dropped_count();
        let loss_rate = if total_count > 0 {
            (dropped_count as f64 / total_count as f64) * 100.0
        } else {
            0.0
        };

        let mut stats_html = format!(
            r#"<div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 15px; margin-bottom: 20px;">
                <div style="display: flex; justify-content: space-between;">
                    <strong>Total Measurements:</strong> 
                    <span style="color: #6c757d;">{total_count}</span>
                </div>
                <div style="display: flex; justify-content: space-between;">
                    <strong>Successful:</strong> 
                    <span style="color: #28a745;">{successful_count}</span>
                </div>
                <div style="display: flex; justify-content: space-between;">
                    <strong>Dropped:</strong> 
                    <span style="color: #dc3545;">{dropped_count}</span>
                </div>
                <div style="display: flex; justify-content: space-between;">
                    <strong>Packet Loss:</strong> 
                    <span style="color: #dc3545;">{loss_rate:.2}%</span>
                </div>"#
        );

        // Add RTT statistics if we have successful measurements
        if successful_count > 0 {
            if let Some(avg) = self.avg_rtt() {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Average RTT:</strong> 
                        <span style="color: #007acc;">{avg:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(min) = self.min_rtt() {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Min RTT:</strong> 
                        <span style="color: #28a745;">{min:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(p25) = self.percentile_rtt(25.0) {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>25th Percentile RTT:</strong> 
                        <span style="color: #fd7e14;">{p25:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(p50) = self.percentile_rtt(50.0) {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Median RTT:</strong> 
                        <span style="color: #fd7e14;">{p50:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(p75) = self.percentile_rtt(75.0) {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>75th Percentile RTT:</strong> 
                        <span style="color: #fd7e14;">{p75:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(max) = self.max_rtt() {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Max RTT:</strong> 
                        <span style="color: #fd7e14;">{max:.2} ms</span>
                    </div>"#
                ));
            }

            if let Some(jitter) = self.rtt_stddev() {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Jitter (Std Dev):</strong> 
                        <span style="color: #6f42c1;">{jitter:.2} ms</span>
                    </div>"#
                ));
            }
        }

        stats_html.push_str("</div>");
        stats_html.push_str(&latency_extras_html(self, None));

        format!(
            r#"<div style="background-color: #f8f9fa; padding: 20px; border-radius: 6px; border-left: 4px solid #007acc;">
                {}
                <div style="margin-top: 15px;">
                    <strong>Timestamp:</strong>
                    <span style="color: #007acc;">{}</span>
                </div>
            </div>"#,
            stats_html,
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )
    }
}

// Implementation for LatencyMeasurement
impl ToHtml for LatencyMeasurement {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        match self.rtt_ms() {
            Some(rtt) => format!(r#"<span style="color: #28a745;">{rtt:.2} ms</span>"#),
            None => r#"<span style="color: #dc3545;">dropped</span>"#.to_string(),
        }
    }
}

// Implementation for Sample (per-chunk throughput observation)
impl ToHtml for Sample {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        match &self.outcome {
            Outcome::Success => format!(
                r#"<div style="display: flex; justify-content: space-between; padding: 8px; background-color: #f8f9fa; border-radius: 4px; margin: 5px 0;">
                        <span>{} in {} ms</span>
                        <span style="color: #6f42c1;">{}</span>
                    </div>"#,
                format_bytes_u64(self.bytes),
                self.duration_us / 1000,
                format_throughput(self.throughput_bps())
            ),
            Outcome::Failure { error, retry_count } => format!(
                r#"<div style="display: flex; justify-content: space-between; padding: 8px; background-color: #f8d7da; border-radius: 4px; margin: 5px 0;">
                        <span style="color: #721c24;">Error: {} (after {} ms, {} retries)</span>
                        <span style="color: #dc3545;">Failed</span>
                    </div>"#,
                error,
                self.duration_us / 1000,
                retry_count
            ),
        }
    }
}

// Implementation for TestType
impl ToHtml for TestType {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        match self {
            TestType::Download => "download".to_string(),
            TestType::Upload => "upload".to_string(),
            TestType::Bidirectional => "bidirectional".to_string(),
            TestType::Simultaneous => "simultaneous".to_string(),
            TestType::FullDuplex => "full-duplex".to_string(),
            TestType::LatencyOnly => "latency-only".to_string(),
            TestType::LatencyUnderLoad => "latency-under-load".to_string(),
        }
    }
}

// Implementation for HttpVersion
impl ToHtml for HttpVersion {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        match self {
            HttpVersion::HTTP1 => "HTTP/1.1".to_string(),
            HttpVersion::H2C => "HTTP/2 Cleartext (h2c)".to_string(),
            HttpVersion::HTTP2 => "HTTP/2 with TLS".to_string(),
            HttpVersion::HTTP3 => "HTTP/3 (QUIC)".to_string(),
        }
    }
}

/// Environment snapshot as a meta grid; mirrors `Environment`'s
/// `Display` impl so terminal and HTML output agree.
fn environment_html(env: &Environment) -> String {
    let mut items = String::new();
    let mut item = |label: &str, value: String| {
        items.push_str(&format!(
            r#"
            <div class="meta-item">
                <span class="meta-label">{label}:</span>
                <span class="meta-value">{value}</span>
            </div>"#,
        ));
    };
    item("Host", escape_html(env.hostname.as_deref().unwrap_or("?")));
    item(
        "OS",
        format!("{} ({})", escape_html(&env.os), escape_html(&env.arch)),
    );
    if let Some(k) = &env.kernel {
        item("Kernel", escape_html(k));
    }
    item("CPUs", env.cpu_count.to_string());
    if let Some(linux) = &env.linux {
        if let Some(cc) = &linux.tcp_congestion_control {
            item("TCP congestion control", escape_html(cc));
        }
        if let (Some(r), Some(w)) = (linux.rmem_max, linux.wmem_max) {
            item("rmem_max / wmem_max", format!("{r} / {w}"));
        }
        if let Some(b) = linux.netdev_max_backlog {
            item("netdev_max_backlog", b.to_string());
        }
    }
    format!(
        r#"
        <div class="section">
            <h2 class="section-title">Environment</h2>
            <div class="meta-info">{items}
            </div>
        </div>"#
    )
}

/// Headline numbers for one row of the suite overview table. Anything a
/// phase did not measure stays `None` and renders as an em dash.
struct PhaseHeadline {
    down_bps: Option<f64>,
    up_bps: Option<f64>,
    p50_ms: Option<f64>,
    loss_pct: Option<f64>,
}

/// Best per-payload-size average, in bits/sec.
fn best_avg_bps(results: &IndexMap<usize, ThroughputResult>) -> Option<f64> {
    results
        .values()
        .map(|r| r.avg_throughput() * 8.0)
        .max_by(f64::total_cmp)
}

fn phase_headline(report: &TestReport) -> PhaseHeadline {
    match &report.result {
        // A bare throughput result carries no direction; surface it in
        // the download column rather than dropping it.
        TestResult::Simple(t) => PhaseHeadline {
            down_bps: Some(t.avg_throughput() * 8.0),
            up_bps: None,
            p50_ms: None,
            loss_pct: None,
        },
        TestResult::Network(net) => {
            let latency = net.latency.as_ref();
            PhaseHeadline {
                down_bps: best_avg_bps(&net.download),
                up_bps: best_avg_bps(&net.upload),
                p50_ms: latency.and_then(|l| l.percentile_rtt(50.0)),
                loss_pct: latency.and_then(|l| {
                    let total = l.count();
                    (total > 0).then(|| l.dropped_count() as f64 / total as f64 * 100.0)
                }),
            }
        }
    }
}

impl ToHtml for PhaseParams {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let payload = match self.payload_size {
            Some(n) => format_bytes_usize(n),
            None => "n/a".to_string(),
        };
        write!(
            writer,
            r#"<div class="params-card">
                <strong>Parameters:</strong>
                payload {payload} &middot; io-unit {io} &middot; connections {conns} &middot; duration {dur}s &middot; type {ty}"#,
            io = format_bytes_usize(self.io_unit),
            conns = self.connections,
            dur = self.duration.as_secs(),
            ty = self.test_type.to_html(),
        )?;
        for note in &self.deviations {
            write!(
                writer,
                r#"
                <div style="color: #fd7e14; margin-top: 4px;">note: {}</div>"#,
                escape_html(note)
            )?;
        }
        write!(writer, "</div>")
    }
}

// Implementation for SuiteReport: a single self-contained document that
// embeds every phase's sections. Streams phase by phase, so a huge
// suite never has to fit in one intermediate String.
impl ToHtml for SuiteReport {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_document_start(writer, "Speed CLI Suite Report")?;

        // Clamps to zero for a never-finalized report (end <= start).
        let duration = (self.end_time - self.start_time)
            .to_std()
            .unwrap_or_default();
        write!(
            writer,
            r#"        <div class="header">
            <h1>═══ Speed CLI Suite Report ═══</h1>
        </div>

        <div class="meta-info">
            <div class="meta-item">
                <span class="meta-label">Version:</span>
                <span class="meta-value">{version}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Server:</span>
                <span class="meta-value">{server}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Start:</span>
                <span class="meta-value">{start}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">End:</span>
                <span class="meta-value">{end}</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Duration:</span>
                <span class="meta-value">{dur:.1}s</span>
            </div>
            <div class="meta-item">
                <span class="meta-label">Phases:</span>
                <span class="meta-value">{phases} completed, {skipped} skipped</span>
            </div>
        </div>
"#,
            version = build_meta_html(&self.build),
            server = escape_html(&self.server),
            start = self.start_time.format("%Y-%m-%d %H:%M:%S UTC"),
            end = self.end_time.format("%Y-%m-%d %H:%M:%S UTC"),
            dur = duration.as_secs_f64(),
            phases = self.reports.len(),
            skipped = self.skipped.len(),
        )?;

        if let Some(env) = &self.environment {
            write!(writer, "{}", environment_html(env))?;
        }

        // Overview: one row per phase, linking to its detail section.
        write!(
            writer,
            r#"
        <div class="section">
            <h2 class="section-title">Overview</h2>"#
        )?;
        if self.reports.is_empty() {
            write!(
                writer,
                r#"
            <div class="banner">No phases completed.</div>"#
            )?;
        } else {
            let dash = "&mdash;".to_string();
            write!(
                writer,
                r#"
            <table class="overview-table">
                <tr><th>Phase</th><th>Type</th><th>Download</th><th>Upload</th><th>p50 RTT</th><th>Loss</th></tr>"#
            )?;
            for (i, nr) in self.reports.iter().enumerate() {
                let h = phase_headline(&nr.report);
                let fmt_bps =
                    |v: Option<f64>| v.map(format_throughput).unwrap_or_else(|| dash.clone());
                let fmt_ms = |v: Option<f64>| {
                    v.map(|m| format!("{m:.2} ms"))
                        .unwrap_or_else(|| dash.clone())
                };
                let fmt_pct = |v: Option<f64>| {
                    v.map(|p| format!("{p:.2}%"))
                        .unwrap_or_else(|| dash.clone())
                };
                write!(
                    writer,
                    r##"
                <tr>
                    <td><a href="#phase-{i}">{label}</a></td>
                    <td>{ty}</td>
                    <td>{down}</td>
                    <td>{up}</td>
                    <td>{p50}</td>
                    <td>{loss}</td>
                </tr>"##,
                    label = escape_html(&nr.label),
                    ty = nr.params.test_type.to_html(),
                    down = fmt_bps(h.down_bps),
                    up = fmt_bps(h.up_bps),
                    p50 = fmt_ms(h.p50_ms),
                    loss = fmt_pct(h.loss_pct),
                )?;
            }
            write!(
                writer,
                r#"
            </table>"#
            )?;
        }
        write!(
            writer,
            r#"
        </div>"#
        )?;

        // Per-phase detail, reusing the single-report sections verbatim.
        // Anchor ids use the index: labels repeat patterns and contain
        // characters that make poor fragment identifiers.
        for (i, nr) in self.reports.iter().enumerate() {
            write!(
                writer,
                r#"
        <div class="phase-section" id="phase-{i}">
            <h2 class="section-title">Phase: {label}</h2>
            "#,
                label = escape_html(&nr.label),
            )?;
            nr.params.write_html(writer)?;
            write_report_sections(&nr.report, writer)?;
            write!(
                writer,
                r#"
        </div>"#
            )?;
        }

        if !self.skipped.is_empty() {
            write!(
                writer,
                r#"
        <div class="section" style="margin-top: 30px;">
            <h2 class="section-title">Skipped Phases</h2>
            <table class="skip-table">"#
            )?;
            for s in &self.skipped {
                write!(
                    writer,
                    r#"
                <tr>
                    <td><strong>{}</strong></td>
                    <td style="color: #dc3545;">{}</td>
                </tr>"#,
                    escape_html(&s.label),
                    escape_html(&s.reason)
                )?;
            }
            write!(
                writer,
                r#"
            </table>
        </div>"#
            )?;
        }

        write_document_end(writer)
    }
}

// Helper functions for formatting
fn format_bytes_usize(bytes: usize) -> String {
    use humansize::{BINARY, format_size};
    format_size(bytes, BINARY)
}

fn format_bytes_u64(bytes: u64) -> String {
    use humansize::{BINARY, format_size};
    format_size(bytes, BINARY)
}

fn format_throughput(bps: f64) -> String {
    use humansize::{BaseUnit, DECIMAL, format_size_i};
    format_size_i(bps, DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"))
}
