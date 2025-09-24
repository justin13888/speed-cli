use crate::performance::http::HttpVersion;
use crate::report::*;
use crate::utils::types::TestType;
use std::io::{self, Write};

// TODO: Expand amount of graphs in HTML
// TODO: Ensure correctness and performance of HTML generation from huge reports (10GB+)

/// Trait for converting structs/enums related to `TestReport` into HTML representation.
///
/// This trait supports both streaming writes to any `Write` implementation and
/// collecting the output as a `String`. This allows for memory-efficient processing
/// of large reports while maintaining backwards compatibility.
///
/// This trait is implemented for all major types in the speed-cli reporting system:
/// - `TestReport` - The main test report structure
/// - `TestConfig` and its variants (`TcpTestConfig`, `UdpTestConfig`, `HttpTestConfig`)
/// - `TestResult` and its variants (`ThroughputResult`, `NetworkTestResult`)
/// - `LatencyResult` and `LatencyMeasurement`
/// - `ThroughputMeasurement`
/// - Enum types like `TestType` and `HttpVersion`
///
/// ## Example
///
/// ```rust
/// use speed_cli::renderer::ToHtml;
/// use speed_cli::report::TestReport;
/// use std::fs::File;
/// use std::io::BufWriter;
///
/// // Assuming you have a TestReport instance
/// let report = /* ... */;
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
        write!(
            writer,
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Speed CLI Test Report</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            line-height: 1.6;
            margin: 0;
            padding: 20px;
            background-color: #f5f5f5;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
            background-color: white;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
            padding: 30px;
        }}
        .header {{
            text-align: center;
            border-bottom: 3px solid #007acc;
            padding-bottom: 20px;
            margin-bottom: 30px;
        }}
        .header h1 {{
            color: #007acc;
            margin: 0;
            font-size: 2.5em;
        }}
        .meta-info {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 15px;
            margin-bottom: 30px;
            padding: 20px;
            background-color: #f8f9fa;
            border-radius: 6px;
        }}
        .meta-item {{
            display: flex;
            justify-content: space-between;
        }}
        .meta-label {{
            font-weight: 600;
            color: #495057;
        }}
        .meta-value {{
            color: #007acc;
            font-weight: 500;
        }}
        .section {{
            margin-bottom: 30px;
        }}
        .section-title {{
            color: #495057;
            border-bottom: 2px solid #e9ecef;
            padding-bottom: 10px;
            margin-bottom: 20px;
            font-size: 1.5em;
            font-weight: 600;
        }}
        .config-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
        }}
        .config-card {{
            background-color: #f8f9fa;
            padding: 20px;
            border-radius: 6px;
            border-left: 4px solid #007acc;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
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

        <div class="section">
            <h2 class="section-title">Configuration</h2>
            <div class="config-grid">
                <div class="config-card">
                    "#,
            self.version,
            self.start_time.format("%Y-%m-%d %H:%M:%S UTC"),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )?;

        self.config.write_html(writer)?;

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

        self.result.write_html(writer)?;

        write!(
            writer,
            r#"
        </div>
    </div>
</body>
</html>"#
        )
    }

    fn to_html(&self) -> String {
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Speed CLI Test Report</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            line-height: 1.6;
            margin: 0;
            padding: 20px;
            background-color: #f5f5f5;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
            background-color: white;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
            padding: 30px;
        }}
        .header {{
            text-align: center;
            border-bottom: 3px solid #007acc;
            padding-bottom: 20px;
            margin-bottom: 30px;
        }}
        .header h1 {{
            color: #007acc;
            margin: 0;
            font-size: 2.5em;
        }}
        .meta-info {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 15px;
            margin-bottom: 30px;
            padding: 20px;
            background-color: #f8f9fa;
            border-radius: 6px;
        }}
        .meta-item {{
            display: flex;
            justify-content: space-between;
        }}
        .meta-label {{
            font-weight: 600;
            color: #495057;
        }}
        .meta-value {{
            color: #007acc;
            font-weight: 500;
        }}
        .section {{
            margin-bottom: 30px;
        }}
        .section-title {{
            color: #495057;
            border-bottom: 2px solid #e9ecef;
            padding-bottom: 10px;
            margin-bottom: 20px;
            font-size: 1.5em;
            font-weight: 600;
        }}
        .config-grid {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
        }}
        .config-card {{
            background-color: #f8f9fa;
            padding: 20px;
            border-radius: 6px;
            border-left: 4px solid #007acc;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
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

        <div class="section">
            <h2 class="section-title">Configuration</h2>
            <div class="config-grid">
                <div class="config-card">
                    {}
                </div>
            </div>
        </div>

        <div class="section">
            <h2 class="section-title">Results</h2>
            {}
        </div>
    </div>
</body>
</html>"#,
            self.version,
            self.start_time.format("%Y-%m-%d %H:%M:%S UTC"),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
            self.config.to_html(),
            self.result.to_html()
        )
    }
}

// Implementation for TestConfig
impl ToHtml for TestConfig {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            TestConfig::Tcp(config) => config.write_html(writer),
            TestConfig::Udp(config) => config.write_html(writer),
            TestConfig::Http(config) => config.write_html(writer),
        }
    }

    fn to_html(&self) -> String {
        match self {
            TestConfig::Tcp(config) => config.to_html(),
            TestConfig::Udp(config) => config.to_html(),
            TestConfig::Http(config) => config.to_html(),
        }
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
            self.total_duration.as_secs_f64(),
            format_throughput(self.avg_throughput()),
            self.measurements.len(),
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
            self.total_duration.as_secs_f64(),
            format_throughput(self.avg_throughput()),
            self.measurements.len(),
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
        };

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
        };

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
                            <div style="margin-left: 20px;">{}</div>
                        </div>"#,
                        format_bytes_usize(*size),
                        result.to_html()
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
                            <div style="margin-left: 20px;">{}</div>
                        </div>"#,
                        format_bytes_usize(*size),
                        result.to_html()
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

            if let Some(jitter) = self.jitter() {
                write!(
                    writer,
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Jitter (Std Dev):</strong> 
                        <span style="color: #6f42c1;">{jitter:.2} ms</span>
                    </div>"#
                )?;
            }
        }

        write!(
            writer,
            r#"</div>
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

            if let Some(jitter) = self.jitter() {
                stats_html.push_str(&format!(
                    r#"<div style="display: flex; justify-content: space-between;">
                        <strong>Jitter (Std Dev):</strong> 
                        <span style="color: #6f42c1;">{jitter:.2} ms</span>
                    </div>"#
                ));
            }
        }

        stats_html.push_str("</div>");

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
        match self.rtt_ms {
            Some(rtt) => format!(r#"<span style="color: #28a745;">{rtt:.2} ms</span>"#),
            None => r#"<span style="color: #dc3545;">dropped</span>"#.to_string(),
        }
    }
}

// Implementation for ThroughputMeasurement
impl ToHtml for ThroughputMeasurement {
    fn write_html<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write!(writer, "{}", self.to_html())
    }

    fn to_html(&self) -> String {
        match self {
            ThroughputMeasurement::Success { bytes, duration } => {
                format!(
                    r#"<div style="display: flex; justify-content: space-between; padding: 8px; background-color: #f8f9fa; border-radius: 4px; margin: 5px 0;">
                        <span>{} in {} ms</span>
                        <span style="color: #6f42c1;">{}</span>
                    </div>"#,
                    format_bytes_u64(*bytes),
                    duration.as_millis(),
                    format_throughput(self.throughput_bps())
                )
            }
            ThroughputMeasurement::Failure {
                error,
                duration,
                retry_count,
            } => {
                format!(
                    r#"<div style="display: flex; justify-content: space-between; padding: 8px; background-color: #f8d7da; border-radius: 4px; margin: 5px 0;">
                        <span style="color: #721c24;">Error: {} (after {} ms, {} retries)</span>
                        <span style="color: #dc3545;">Failed</span>
                    </div>"#,
                    error,
                    duration.as_millis(),
                    retry_count
                )
            }
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
            TestType::LatencyOnly => "latency-only".to_string(),
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
