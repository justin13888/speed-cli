use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    TestType,
    constants::{
        DEFAULT_HTTP_PACKET_SIZES, DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DEFAULT_TCP_PACKET_SIZES,
        DEFAULT_TCP_PORT, DEFAULT_UDP_PORT,
    },
    speed::http::HttpVersion,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub payload_sizes: Vec<usize>,
}

impl TcpTestConfig {
    pub fn new(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        payload_sizes: Vec<usize>,
    ) -> Self {
        Self {
            server,
            port: port.unwrap_or(DEFAULT_TCP_PORT), // Default TCP port
            duration: Duration::from_secs(duration),
            parallel_connections: parallel_connections.max(1),
            payload_sizes: if payload_sizes.is_empty() {
                DEFAULT_TCP_PACKET_SIZES.to_vec()
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
    pub payload_sizes: Vec<usize>,
}

impl UdpTestConfig {
    pub fn new(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_streams: usize,
        payload_sizes: Vec<usize>,
    ) -> Self {
        Self {
            server,
            port: port.unwrap_or(DEFAULT_UDP_PORT), // Default UDP port
            duration,
            parallel_streams: parallel_streams.max(1),
            payload_sizes: if payload_sizes.is_empty() {
                DEFAULT_TCP_PACKET_SIZES.to_vec() // Use TCP sizes as default for UDP
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
    pub duration: u64,
    /// Number of parallel connections
    pub parallel_connections: usize,
    pub test_type: TestType,
    /// Payload sizes for testing.
    pub payload_sizes: Vec<usize>,
    pub http_version: HttpVersion,
}

impl HttpTestConfig {
    pub fn new(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        payload_sizes: Vec<usize>,
        http_version: HttpVersion,
    ) -> Self {
        let is_secure = match http_version {
            HttpVersion::HTTP1 | HttpVersion::H2C => true,
            HttpVersion::HTTP2 | HttpVersion::HTTP3 => true,
        };
        let scheme = if is_secure { "https" } else { "http" };
        let port = port.unwrap_or(if is_secure {
            DEFAULT_HTTP_PORT
        } else {
            DEFAULT_HTTPS_PORT
        });
        let server_url = format!("{scheme}://{server}:{port}");

        let payload_sizes = if payload_sizes.is_empty() {
            DEFAULT_HTTP_PACKET_SIZES.to_vec()
        } else {
            payload_sizes
        };
        Self {
            server_url,
            duration,
            parallel_connections: parallel_connections.max(1),
            test_type,
            payload_sizes,
            http_version,
        }
    }
}
