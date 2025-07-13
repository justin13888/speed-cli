use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::utils::format::format_bytes;
use crate::{
    TestType,
    constants::{
        DEFAULT_HTTP_PACKET_SIZES, DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DEFAULT_TCP_PACKET_SIZES,
        DEFAULT_TCP_PORT, DEFAULT_UDP_PACKET_SIZES, DEFAULT_UDP_PORT,
    },
    performance::http::HttpVersion,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestConfig {
    Tcp(TcpTestConfig),
    Udp(UdpTestConfig),
    Http(HttpTestConfig),
}

impl From<TcpTestConfig> for TestConfig {
    fn from(config: TcpTestConfig) -> Self {
        TestConfig::Tcp(config)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpTestConfig {
    pub server: String,
    pub port: u16,
    pub duration: Duration,
    /// Number of parallel TCP connections
    pub parallel_connections: usize,
    /// Payload sizes to use for the test, in bytes. Note this doesn't make sense for TCP but included anyways.
    pub payload_sizes: IndexSet<usize>,
}

impl TcpTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
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
            payload_sizes: if payload_sizes.is_empty() {
                IndexSet::from_iter(DEFAULT_TCP_PACKET_SIZES.iter().copied())
            } else {
                payload_sizes
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpTestConfig {
    pub server: String,
    pub port: u16,
    pub duration: u64,
    /// Number of parallel UDP streams. This is somewhat less relevant for UDP but included for consistency.
    pub parallel_streams: usize,
    /// Payload sizes to use for the test, in bytes.
    pub payload_sizes: IndexSet<usize>,
}

impl UdpTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_streams: usize,
        payload_sizes: T,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        Self {
            server,
            port: port.unwrap_or(DEFAULT_UDP_PORT), // Default UDP port
            duration,
            parallel_streams: parallel_streams.max(1),
            payload_sizes: if payload_sizes.is_empty() {
                IndexSet::from_iter(DEFAULT_UDP_PACKET_SIZES.iter().copied())
            } else {
                payload_sizes
            },
        }
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
}

impl HttpTestConfig {
    pub fn new<T>(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        payload_sizes: T,
        http_version: HttpVersion,
    ) -> Self
    where
        T: IntoIterator<Item = usize>,
    {
        let payload_sizes: IndexSet<usize> = payload_sizes.into_iter().collect();
        let is_secure = match http_version {
            HttpVersion::HTTP1 | HttpVersion::H2C => false,
            HttpVersion::HTTP2 | HttpVersion::HTTP3 => true,
        };
        let scheme = if is_secure { "https" } else { "http" };
        let port = port.unwrap_or(if is_secure {
            DEFAULT_HTTPS_PORT
        } else {
            DEFAULT_HTTP_PORT
        });
        let server_url = format!("{scheme}://{server}:{port}");

        let payload_sizes = if payload_sizes.is_empty() {
            IndexSet::from_iter(DEFAULT_HTTP_PACKET_SIZES.iter().copied())
        } else {
            payload_sizes
        };
        Self {
            server_url,
            duration: Duration::from_secs(duration),
            parallel_connections: parallel_connections.max(1),
            test_type,
            payload_sizes,
            http_version,
        }
    }
}

impl Display for TestConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            TestConfig::Tcp(config) => write!(f, "{config}"),
            TestConfig::Udp(config) => write!(f, "{config}"),
            TestConfig::Http(config) => write!(f, "{config}"),
        }
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
            "HTTP".green()
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
