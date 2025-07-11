use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::{TestType, speed::http::HttpVersion};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestConfig {
    TCP(TcpTestConfig),
    UDP(UdpTestConfig),
    HTTP(HttpTestConfig),
}

impl From<TcpTestConfig> for TestConfig {
    fn from(config: TcpTestConfig) -> Self {
        TestConfig::TCP(config)
    }
}

impl From<UdpTestConfig> for TestConfig {
    fn from(config: UdpTestConfig) -> Self {
        TestConfig::UDP(config)
    }
}

impl From<HttpTestConfig> for TestConfig {
    fn from(config: HttpTestConfig) -> Self {
        TestConfig::HTTP(config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpTestConfig {
    pub server_addr: String,
    pub port: u16,
    pub duration: u64,
}

impl TcpTestConfig {
    pub fn new(server_addr: String, port: Option<u16>, duration: u64) -> Self {
        Self {
            server_addr,
            port: port.unwrap_or(5201), // Default TCP port
            duration,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpTestConfig {
    pub server_addr: String,
    pub port: u16,
    pub duration: u64,
    pub target_bandwidth: f64, // TODO: get rid of this field
}

impl UdpTestConfig {
    pub fn new(server_addr: String, port: Option<u16>, duration: u64) -> Self {
        Self {
            server_addr,
            port: port.unwrap_or(5201), // Default UDP port
            duration,
            target_bandwidth: 100.0,
        } // TODO: Verify all attributes are being used
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestConfig {
    /// Server URL (e.g., http://example.com)
    pub server_url: String,
    pub duration: u64,
    pub parallel_connections: usize,
    pub test_type: TestType,
    pub http_version: HttpVersion,
    pub test_sizes: Vec<usize>, // Test with different payload sizes
    pub adaptive_sizing: bool,
}
// TODO: Update these ^^

impl HttpTestConfig {
    pub fn new(
        server: String,
        port: Option<u16>,
        duration: u64,
        parallel_connections: usize,
        test_type: TestType,
        http_version: HttpVersion,
        test_sizes: Vec<usize>,
        adaptive_sizing: bool,
    ) -> Self {
        let is_secure = match http_version {
            HttpVersion::HTTP1 | HttpVersion::H2C => true,
            HttpVersion::HTTP2 | HttpVersion::HTTP3 => true,
        };
        let scheme = if is_secure { "https" } else { "http" };
        // TODO: Validate server value (if necessary)
        let port = port.unwrap_or(if is_secure { 8080 } else { 8443 });
        let server_url = format!("{scheme}://{server}:{port}");

        Self {
            server_url,
            duration,
            parallel_connections,
            test_type,
            http_version,
            test_sizes,
            adaptive_sizing,
        }
    }
}
