use serde::{Deserialize, Serialize};

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum ClientMode {
    /// TCP
    TCP,
    /// UDP
    UDP,
    /// Raw QUIC stream throughput
    QUIC,
    /// HTTP/1.1 without TLS
    HTTP1,
    /// h2c (HTTP/2 Cleartext)
    H2C,
    /// HTTP/2 with TLS
    HTTP2,
    /// HTTP/3 (over QUIC)
    HTTP3,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum TestType {
    /// Download only
    Download,
    /// Upload only
    Upload,
    /// Bidirectional (both download and upload)
    #[default]
    Bidirectional,
    /// Simultaneous download and upload (separate connections, run in parallel)
    Simultaneous,
    /// Full duplex on a single connection (TCP only)
    FullDuplex,
    /// Latency only
    #[clap(alias = "latency")]
    LatencyOnly,
    /// Latency measured while the link is saturated — a WiFi / bufferbloat
    /// stress test. Captures an idle baseline first, then probes latency at a
    /// high rate under load so spikes from the WiFi card / AP become visible.
    /// UDP only.
    #[clap(alias = "latency-load", alias = "wifi")]
    LatencyUnderLoad,
}

/// Congestion-control algorithm for the QUIC-based transports (raw QUIC
/// and HTTP/3). TCP-based protocols always use the OS default — there
/// is no portable per-socket TCP knob — so this never applies to them.
/// CUBIC is quinn's (and the internet's) default; BBR is the opt-in
/// alternative for high-BDP or lossy paths.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum CongestionAlgorithm {
    #[default]
    Cubic,
    Bbr,
}

impl fmt::Display for CongestionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CongestionAlgorithm::Cubic => write!(f, "cubic"),
            CongestionAlgorithm::Bbr => write!(f, "bbr"),
        }
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
            TestType::FullDuplex => write!(f, "full-duplex"),
            TestType::LatencyOnly => write!(f, "latency-only"),
            TestType::LatencyUnderLoad => write!(f, "latency-under-load"),
        }
    }
}
