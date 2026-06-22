use std::fmt::{self, Display, Formatter};

use colored::Colorize as _;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    report::{
        LatencyResult, STANDARD_MTU, ThroughputAccounting, ThroughputResult,
        WIRE_OVERHEAD_TCP_BYTES, WIRE_OVERHEAD_UDP_BYTES,
    },
    utils::format::format_bytes,
};

fn default_accounting() -> ThroughputAccounting {
    ThroughputAccounting::Goodput
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkTestResult {
    pub latency: Option<LatencyResult>,
    /// Map of download results by payload size
    pub download: IndexMap<usize, ThroughputResult>,
    /// Map of upload results by payload size
    pub upload: IndexMap<usize, ThroughputResult>,
    /// Protocol type for display purposes
    pub protocol: NetworkProtocol,
    /// Whether throughput numbers should be reported as goodput
    /// (application bytes only) or wire-rate (with TCP/IP or UDP/IP
    /// framing overhead added back in).
    #[serde(default = "default_accounting")]
    pub accounting: ThroughputAccounting,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NetworkProtocol {
    Http,
    Tcp,
    Udp,
    /// Raw QUIC stream throughput (the QUIC analog of `Tcp`).
    Quic,
}

impl NetworkTestResult {
    pub fn new_http() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Http,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_tcp() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Tcp,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_udp() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Udp,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_quic() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Quic,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn with_accounting(mut self, accounting: ThroughputAccounting) -> Self {
        self.accounting = accounting;
        self
    }

    /// Wire-rate framing overhead per segment for this protocol, IPv4.
    pub fn wire_overhead_per_segment(&self) -> usize {
        match self.protocol {
            NetworkProtocol::Tcp | NetworkProtocol::Http => WIRE_OVERHEAD_TCP_BYTES,
            NetworkProtocol::Udp | NetworkProtocol::Quic => WIRE_OVERHEAD_UDP_BYTES,
        }
    }

    /// MTU used to estimate segment count from payload size.
    pub fn wire_mtu(&self) -> usize {
        STANDARD_MTU
    }
}

impl NetworkTestResult {
    /// Format a one-line wire-rate annotation that the renderer can append
    /// after each per-payload throughput block when accounting is set to
    /// Wire (or as a footnote in Goodput mode).
    fn render_wire_rate_line(&self, result: &ThroughputResult) -> String {
        use humansize::{BaseUnit, DECIMAL, format_size_i};
        let bps = result.avg_throughput_wire_bps(self.wire_overhead_per_segment(), self.wire_mtu());
        format!(
            "    Wire-rate (est): {}",
            format_size_i(bps, DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"))
        )
    }
}

impl Display for NetworkTestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let protocol_prefix = match self.protocol {
            NetworkProtocol::Http => "HTTP ",
            NetworkProtocol::Tcp => "TCP ",
            NetworkProtocol::Udp => "UDP ",
            NetworkProtocol::Quic => "QUIC ",
        };

        // Display latency if available
        if let Some(latency) = &self.latency {
            writeln!(
                f,
                "  {}",
                format!("{}Latency Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
            write!(f, "{latency}")?;
            writeln!(f)?;
        }

        // Display download results
        if !self.download.is_empty() {
            writeln!(
                f,
                "  {}",
                format!("{}Download Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
            for (size, result) in &self.download {
                writeln!(
                    f,
                    "    {} ({}):",
                    "Payload Size".bright_blue(),
                    format_bytes(*size).yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{result}");
                for line in result_str.lines() {
                    writeln!(f, "    {line}")?;
                }
                if matches!(self.accounting, ThroughputAccounting::Wire) {
                    writeln!(f, "{}", self.render_wire_rate_line(result))?;
                }
            }
        }

        // Display upload results
        if !self.upload.is_empty() {
            writeln!(
                f,
                "  {}",
                format!("{}Upload Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
            for (size, result) in &self.upload {
                writeln!(
                    f,
                    "    {} ({}):",
                    "Payload Size".bright_blue(),
                    format_bytes(*size).yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{result}");
                for line in result_str.lines() {
                    writeln!(f, "    {line}")?;
                }
                if matches!(self.accounting, ThroughputAccounting::Wire) {
                    writeln!(f, "{}", self.render_wire_rate_line(result))?;
                }
            }
        }

        Ok(())
    }
}
