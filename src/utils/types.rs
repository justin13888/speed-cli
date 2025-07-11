use serde::{Deserialize, Serialize};

#[derive(clap::ValueEnum, Clone, Debug)]
#[clap(rename_all = "lowercase")]
pub enum ClientMode {
    /// TCP
    TCP,
    /// UDP
    UDP,
    /// HTTP/1.1 without TLS
    HTTP1,
    /// h2c (HTTP/2 Cleartext)
    H2C,
    /// HTTP/2 with TLS
    HTTP2,
    /// HTTP/3 (QUIC)
    HTTP3,
}

#[derive(Debug, Clone, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum TestType {
    /// Download only
    Download,
    /// Upload only
    Upload,
    /// Bidirectional (both download and upload)
    Bidirectional,
    /// Simultaneous download and upload
    Simultaneous,
    /// Latency only
    LatencyOnly,
}

impl Default for TestType {
    fn default() -> Self {
        TestType::Bidirectional
    }
}

use std::fmt;
impl fmt::Display for TestType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestType::Download => write!(f, "download"),
            TestType::Upload => write!(f, "upload"),
            TestType::Bidirectional => write!(f, "bidirectional"),
            TestType::Simultaneous => write!(f, "simultaneous"),
            TestType::LatencyOnly => write!(f, "latency-only"),
        }
    }
}
