use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::constants::DEFAULT_CHUNK_SIZE;
use crate::report::ThroughputAccounting;
use crate::utils::format::format_bytes;
use crate::{
    CongestionAlgorithm, TestType,
    constants::{
        DEFAULT_HTTP_PAYLOAD_SIZES, DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT,
        DEFAULT_TCP_PAYLOAD_SIZES, DEFAULT_TCP_PORT, DEFAULT_UDP_PAYLOAD_SIZES, DEFAULT_UDP_PORT,
        MAX_UDP_PAYLOAD_SIZE,
    },
    performance::http::HttpVersion,
};

fn default_accounting() -> ThroughputAccounting {
    ThroughputAccounting::Goodput
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestConfig {
    Tcp(TcpTestConfig),
    Udp(UdpTestConfig),
    Http(HttpTestConfig),
    Quic(QuicTestConfig),
}

impl From<TcpTestConfig> for TestConfig {
    fn from(config: TcpTestConfig) -> Self {
        TestConfig::Tcp(config)
    }
}

impl From<QuicTestConfig> for TestConfig {
    fn from(config: QuicTestConfig) -> Self {
        TestConfig::Quic(config)
    }
}

impl From<UdpTestConfig> for TestConfig {
    fn from(config: UdpTestConfig) -> Self {
        TestConfig::Udp(config)
    }
}

impl From<HttpTestConfig> for TestConfig {
    fn from(config: HttpTestConfig) -> Self {
        TestConfig::Http(config)
    }
}

/// Default TCP client read buffer size (128 KB) - matches the server-side buffer
/// and avoids the syscall amplification you get when sizing the buffer to a
/// small payload.
pub const DEFAULT_TCP_READ_BUFFER: usize = 131_072;

/// Default warmup duration. Samples taken during the warmup are discarded so
/// they don't contaminate the steady-state numbers with TCP slow-start, TLS
/// handshake jitter, or connection-pool warmup.
pub const DEFAULT_WARMUP: Duration = Duration::from_secs(1);

fn default_warmup() -> Duration {
    DEFAULT_WARMUP
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpTestConfig {
    pub server: String,
    pub port: u16,
    pub duration: Duration,
    /// Number of parallel TCP connections
    pub parallel_connections: usize,
    pub test_type: TestType,
    /// Payload sizes to use for the test, in bytes. Note this doesn't make sense for TCP but included anyways.
    pub payload_sizes: IndexSet<usize>,
    /// Size of the per-connection read buffer in bytes. A larger buffer means
    /// fewer syscalls per second; the default (128 KB) matches the server.
    #[serde(default = "default_tcp_read_buffer")]
    pub read_buffer_size: usize,
    /// Discard samples taken during this initial window. Counted against
    /// `duration` (not added on top of it).
    #[serde(default = "default_warmup")]
    pub warmup: Duration,
    /// Goodput vs wire-rate accounting.
    #[serde(default = "default_accounting")]
    pub accounting: ThroughputAccounting,
    /// Fixed SO_RCVBUF/SO_SNDBUF, in bytes, applied before connect.
    /// `None` (default) keeps kernel autotuning, which is usually
    /// better for TCP — a fixed size disables Linux receive autotuning.
    #[serde(default)]
    pub socket_buffer: Option<usize>,
}

fn default_tcp_read_buffer() -> usize {
    DEFAULT_TCP_READ_BUFFER
}

impl TcpTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        payload_sizes: T,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        Self {
            server,
            port: port.unwrap_or(DEFAULT_TCP_PORT), // Default TCP port
            duration: Duration::from_secs(duration),
            parallel_connections: parallel_connections.max(1),
            test_type,
            payload_sizes: if payload_sizes.is_empty() {
                IndexSet::from_iter(DEFAULT_TCP_PAYLOAD_SIZES.iter().copied())
            } else {
                payload_sizes
            },
            read_buffer_size: DEFAULT_TCP_READ_BUFFER,
            warmup: DEFAULT_WARMUP,
            accounting: ThroughputAccounting::Goodput,
            socket_buffer: None,
        }
    }

    pub fn with_warmup(mut self, warmup: Duration) -> Self {
        self.warmup = warmup;
        self
    }

    pub fn with_accounting(mut self, accounting: ThroughputAccounting) -> Self {
        self.accounting = accounting;
        self
    }

    pub fn with_socket_buffer(mut self, socket_buffer: Option<usize>) -> Self {
        self.socket_buffer = socket_buffer;
        self
    }
}

/// Raw-QUIC stream test configuration. Mirrors [`TcpTestConfig`] — the
/// raw-QUIC test is the QUIC analog of the raw-TCP test, with the same
/// `'U'`/`'D'`/`'F'`/`'P'`/`'H'` command shape over QUIC bidirectional
/// streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicTestConfig {
    pub server: String,
    pub port: u16,
    pub duration: Duration,
    /// Number of parallel QUIC bidirectional streams.
    pub parallel_connections: usize,
    pub test_type: TestType,
    pub payload_sizes: IndexSet<usize>,
    #[serde(default = "default_tcp_read_buffer")]
    pub read_buffer_size: usize,
    #[serde(default = "default_warmup")]
    pub warmup: Duration,
    #[serde(default = "default_accounting")]
    pub accounting: ThroughputAccounting,
    /// Congestion controller the QUIC transport ran with. Recorded so
    /// reports are comparable; back-fills to cubic for reports written
    /// before the knob existed (correct — cubic was the only option).
    #[serde(default)]
    pub congestion: CongestionAlgorithm,
}

impl QuicTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        payload_sizes: T,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        Self {
            server,
            port: port.unwrap_or(DEFAULT_TCP_PORT),
            duration: Duration::from_secs(duration),
            parallel_connections: parallel_connections.max(1),
            test_type,
            payload_sizes: if payload_sizes.is_empty() {
                IndexSet::from_iter(DEFAULT_TCP_PAYLOAD_SIZES.iter().copied())
            } else {
                payload_sizes
            },
            read_buffer_size: DEFAULT_TCP_READ_BUFFER,
            warmup: DEFAULT_WARMUP,
            accounting: ThroughputAccounting::Goodput,
            congestion: CongestionAlgorithm::default(),
        }
    }

    pub fn with_warmup(mut self, warmup: Duration) -> Self {
        self.warmup = warmup;
        self
    }

    pub fn with_accounting(mut self, accounting: ThroughputAccounting) -> Self {
        self.accounting = accounting;
        self
    }

    pub fn with_congestion(mut self, congestion: CongestionAlgorithm) -> Self {
        self.congestion = congestion;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpTestConfig {
    pub server: String,
    pub port: u16,
    pub duration: u64,
    /// Number of parallel UDP streams. This is somewhat less relevant for UDP but included for consistency.
    pub parallel_streams: usize,
    pub test_type: TestType,
    /// Payload sizes to use for the test, in bytes.
    pub payload_sizes: IndexSet<usize>,
    /// Discard samples taken during this initial window.
    #[serde(default = "default_warmup")]
    pub warmup: Duration,
    /// Goodput vs wire-rate accounting.
    #[serde(default = "default_accounting")]
    pub accounting: ThroughputAccounting,
    /// Target send rate in bits per second. Zero means "saturate" (the
    /// blaster will send as fast as the runtime allows). For accurate
    /// pacing above ~100 Mbps the kernel sleep granularity becomes the
    /// limiting factor; this is a known issue called out in the
    /// roadmap.
    #[serde(default)]
    pub target_rate_bps: u64,
    /// Per-direction UDP socket buffer size override, in bytes. `None`
    /// (default) uses the enlarged 8 MiB default — UDP has no kernel
    /// autotuning, so a large buffer is always right.
    #[serde(default)]
    pub socket_buffer: Option<usize>,
}

impl UdpTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_streams: usize,
        test_type: TestType,
        payload_sizes: T,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        // A UDP DATA payload above MAX_UDP_PAYLOAD_SIZE makes the datagram
        // exceed the 65507 B IPv4 limit, so every send fails with EMSGSIZE and
        // 0 packets reach the wire. Clamp any such size down to the max (the
        // IndexSet dedups any collisions) and warn, rather than running a test
        // that silently sends nothing.
        let payload_sizes: IndexSet<usize> = if payload_sizes.is_empty() {
            IndexSet::from_iter(DEFAULT_UDP_PAYLOAD_SIZES.iter().copied())
        } else {
            payload_sizes
                .into_iter()
                .map(|sz| {
                    if sz > MAX_UDP_PAYLOAD_SIZE {
                        tracing::warn!(
                            "UDP payload size {sz} B exceeds the single-datagram maximum \
                             ({MAX_UDP_PAYLOAD_SIZE} B = 65507 IPv4 UDP max − 24 B header); \
                             clamping to {MAX_UDP_PAYLOAD_SIZE} B"
                        );
                        MAX_UDP_PAYLOAD_SIZE
                    } else {
                        sz
                    }
                })
                .collect()
        };
        Self {
            server,
            port: port.unwrap_or(DEFAULT_UDP_PORT), // Default UDP port
            duration,
            parallel_streams: parallel_streams.max(1),
            test_type,
            payload_sizes,
            warmup: DEFAULT_WARMUP,
            accounting: ThroughputAccounting::Goodput,
            target_rate_bps: 0,
            socket_buffer: None,
        }
    }

    pub fn with_warmup(mut self, warmup: Duration) -> Self {
        self.warmup = warmup;
        self
    }

    pub fn with_accounting(mut self, accounting: ThroughputAccounting) -> Self {
        self.accounting = accounting;
        self
    }

    pub fn with_target_rate_bps(mut self, target_rate_bps: u64) -> Self {
        self.target_rate_bps = target_rate_bps;
        self
    }

    pub fn with_socket_buffer(mut self, socket_buffer: Option<usize>) -> Self {
        self.socket_buffer = socket_buffer;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestConfig {
    /// Server URL (e.g., http://192.168.1.100, https://example.com) including port if necessary
    pub server_url: String,
    /// Test duration in seconds
    pub duration: Duration,
    /// Number of parallel connections
    pub parallel_connections: usize,
    pub test_type: TestType,
    /// Payload sizes for testing.
    pub payload_sizes: IndexSet<usize>,
    pub http_version: HttpVersion,
    /// Maximum chunk size for HTTP requests. This is effective only for HTTP/1.1 tests.
    pub chunk_size: usize,
    /// Discard samples taken during this initial window.
    #[serde(default = "default_warmup")]
    pub warmup: Duration,
    /// Goodput vs wire-rate accounting.
    #[serde(default = "default_accounting")]
    pub accounting: ThroughputAccounting,
    /// Congestion controller for the QUIC transport. Meaningful for
    /// HTTP/3 only; TCP-backed versions always use the OS controller
    /// and ignore this. Back-fills to cubic for old reports.
    #[serde(default)]
    pub congestion: CongestionAlgorithm,
}

impl HttpTestConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        payload_sizes: T,
        chunk_size: Option<usize>,
        http_version: HttpVersion,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        let scheme = http_version.scheme();
        let port = port.unwrap_or(if http_version.is_secure() {
            DEFAULT_HTTPS_PORT
        } else {
            DEFAULT_HTTP_PORT
        });
        let server_url = format!("{scheme}://{server}:{port}");

        let payload_sizes = if payload_sizes.is_empty() {
            IndexSet::from_iter(DEFAULT_HTTP_PAYLOAD_SIZES.iter().copied())
        } else {
            payload_sizes
        };
        Self {
            server_url,
            duration: Duration::from_secs(duration),
            parallel_connections: parallel_connections.max(1),
            test_type,
            payload_sizes,
            chunk_size: chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
            http_version,
            warmup: DEFAULT_WARMUP,
            accounting: ThroughputAccounting::Goodput,
            congestion: CongestionAlgorithm::default(),
        }
    }

    pub fn with_warmup(mut self, warmup: Duration) -> Self {
        self.warmup = warmup;
        self
    }

    pub fn with_accounting(mut self, accounting: ThroughputAccounting) -> Self {
        self.accounting = accounting;
        self
    }

    pub fn with_congestion(mut self, congestion: CongestionAlgorithm) -> Self {
        self.congestion = congestion;
        self
    }
}

impl Display for TestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TestConfig::Tcp(config) => write!(f, "{config}"),
            TestConfig::Udp(config) => write!(f, "{config}"),
            TestConfig::Http(config) => write!(f, "{config}"),
            TestConfig::Quic(config) => write!(f, "{config}"),
        }
    }
}

impl Display for QuicTestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Protocol".bright_blue().bold(),
            "QUIC".green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Server".bright_blue().bold(),
            self.server.cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Port".bright_blue().bold(),
            self.port.to_string().yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_blue().bold(),
            format!("{}s", self.duration.as_secs()).magenta()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Parallel Streams".bright_blue().bold(),
            self.parallel_connections.to_string().green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Congestion Control".bright_blue().bold(),
            self.congestion.to_string().yellow()
        )?;

        let sizes: Vec<String> = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes(*s))
            .collect();
        writeln!(
            f,
            "  {}: [{}]",
            "Payload Sizes".bright_blue().bold(),
            sizes.join(", ").white()
        )?;

        Ok(())
    }
}

impl Display for TcpTestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Protocol".bright_blue().bold(),
            "TCP".green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Server".bright_blue().bold(),
            self.server.cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Port".bright_blue().bold(),
            self.port.to_string().yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_blue().bold(),
            format!("{}s", self.duration.as_secs()).magenta()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Parallel Connections".bright_blue().bold(),
            self.parallel_connections.to_string().green()
        )?;

        let sizes: Vec<String> = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes(*s))
            .collect();
        writeln!(
            f,
            "  {}: [{}]",
            "Payload Sizes".bright_blue().bold(),
            sizes.join(", ").white()
        )?;

        Ok(())
    }
}

impl Display for UdpTestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Protocol".bright_blue().bold(),
            "UDP".green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Server".bright_blue().bold(),
            self.server.cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Port".bright_blue().bold(),
            self.port.to_string().yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_blue().bold(),
            format!("{}s", self.duration).magenta()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Parallel Streams".bright_blue().bold(),
            self.parallel_streams.to_string().green()
        )?;

        let sizes: Vec<String> = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes(*s))
            .collect();
        writeln!(
            f,
            "  {}: [{}]",
            "Payload Sizes".bright_blue().bold(),
            sizes.join(", ").white()
        )?;

        Ok(())
    }
}

impl Display for HttpTestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Protocol".bright_blue().bold(),
            self.http_version
                .scheme()
                .to_string()
                .to_uppercase()
                .green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Server URL".bright_blue().bold(),
            self.server_url.cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_blue().bold(),
            format!("{}s", self.duration.as_secs()).magenta()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Parallel Connections".bright_blue().bold(),
            self.parallel_connections.to_string().green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Test Type".bright_blue().bold(),
            format!("{:?}", self.test_type).yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "HTTP Version".bright_blue().bold(),
            format!("{:?}", self.http_version).yellow()
        )?;
        // Congestion control is a QUIC property; showing it for
        // TCP-backed versions would imply a knob that does not exist.
        if matches!(self.http_version, HttpVersion::HTTP3) {
            writeln!(
                f,
                "  {}: {}",
                "Congestion Control".bright_blue().bold(),
                self.congestion.to_string().yellow()
            )?;
        }

        let sizes: Vec<String> = self
            .payload_sizes
            .iter()
            .map(|s| format_bytes(*s))
            .collect();
        writeln!(
            f,
            "  {}: [{}]",
            "Payload Sizes".bright_blue().bold(),
            sizes.join(", ").white()
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn udp_config(sizes: Vec<usize>) -> UdpTestConfig {
        UdpTestConfig::new("host".into(), None, 1, 1, TestType::Download, sizes)
    }

    #[test]
    fn udp_defaults_fit_a_single_datagram() {
        let sizes = udp_config(vec![]).payload_sizes;
        assert!(sizes.iter().all(|&s| s <= MAX_UDP_PAYLOAD_SIZE));
        assert!(sizes.contains(&MAX_UDP_PAYLOAD_SIZE));
        // The old 64 KiB default exceeded the datagram limit and must be gone.
        assert!(!sizes.contains(&65536));
    }

    #[test]
    fn udp_clamps_oversized_user_sizes() {
        let sizes = udp_config(vec![1024, 65536, 70000]).payload_sizes;
        // Valid size is untouched; both oversized values collapse onto the cap.
        assert!(sizes.contains(&1024));
        assert!(sizes.contains(&MAX_UDP_PAYLOAD_SIZE));
        assert!(sizes.iter().all(|&s| s <= MAX_UDP_PAYLOAD_SIZE));
        assert_eq!(sizes.len(), 2);
    }
}
