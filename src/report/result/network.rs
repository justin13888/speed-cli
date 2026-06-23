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
    /// Latency sampled while the link was saturated — the under-load phase of a
    /// `latency-under-load` stress test. `latency` holds the idle baseline
    /// captured first; comparing the two surfaces bufferbloat and the WiFi
    /// card / AP latency spikes that only appear under load.
    #[serde(default)]
    pub latency_under_load: Option<LatencyResult>,
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
            latency_under_load: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Http,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_tcp() -> Self {
        Self {
            latency: None,
            latency_under_load: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Tcp,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_udp() -> Self {
        Self {
            latency: None,
            latency_under_load: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Udp,
            accounting: ThroughputAccounting::Goodput,
        }
    }

    pub fn new_quic() -> Self {
        Self {
            latency: None,
            latency_under_load: None,
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

    /// Bufferbloat snapshot: idle-vs-loaded median and p99 RTT. `Some` only
    /// when both an idle baseline (`latency`) and an under-load series
    /// (`latency_under_load`) carry enough data. The growth from idle to
    /// loaded is the headline WiFi / AP latency-under-load signal.
    pub fn bufferbloat_inflation(&self) -> Option<BufferbloatInflation> {
        let idle = self.latency.as_ref()?;
        let loaded = self.latency_under_load.as_ref()?;
        Some(BufferbloatInflation {
            idle_p50: idle.percentile_rtt(50.0)?,
            idle_p99: idle.percentile_rtt(99.0)?,
            loaded_p50: loaded.percentile_rtt(50.0)?,
            loaded_p99: loaded.percentile_rtt(99.0)?,
        })
    }
}

/// Idle-vs-under-load latency snapshot for the bufferbloat comparison.
#[derive(Debug, Clone, Copy)]
pub struct BufferbloatInflation {
    pub idle_p50: f64,
    pub idle_p99: f64,
    pub loaded_p50: f64,
    pub loaded_p99: f64,
}

impl BufferbloatInflation {
    /// Median RTT growth from idle to under load, in milliseconds.
    pub fn d_median_ms(&self) -> f64 {
        self.loaded_p50 - self.idle_p50
    }

    /// p99 RTT growth from idle to under load, in milliseconds.
    pub fn d_p99_ms(&self) -> f64 {
        self.loaded_p99 - self.idle_p99
    }

    /// Severe bufferbloat: p99 grew by 100 ms or more under load.
    pub fn is_severe(&self) -> bool {
        self.d_p99_ms() >= 100.0
    }

    /// Mild bufferbloat: p99 grew by 30 ms or more under load.
    pub fn is_mild(&self) -> bool {
        self.d_p99_ms() >= 30.0
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
            let title = if self.latency_under_load.is_some() {
                format!("{protocol_prefix}Latency (idle baseline):")
            } else {
                format!("{protocol_prefix}Latency Results:")
            };
            writeln!(f, "  {}", title.bright_green().bold())?;
            write!(f, "{latency}")?;
            writeln!(f)?;
        }

        // Latency under load (WiFi / bufferbloat stress test).
        if let Some(loaded) = &self.latency_under_load {
            writeln!(
                f,
                "  {}",
                format!("{protocol_prefix}Latency Under Load:")
                    .bright_green()
                    .bold()
            )?;
            write!(f, "{loaded}")?;
            writeln!(f)?;
            if let Some(inf) = self.bufferbloat_inflation() {
                let line = format!(
                    "    Bufferbloat: median {dm:+.1} ms, p99 {dp:+.1} ms under load \
                     (idle {i50:.1}/{i99:.1} → loaded {l50:.1}/{l99:.1} ms)",
                    dm = inf.d_median_ms(),
                    dp = inf.d_p99_ms(),
                    i50 = inf.idle_p50,
                    i99 = inf.idle_p99,
                    l50 = inf.loaded_p50,
                    l99 = inf.loaded_p99,
                );
                let coloured = if inf.is_severe() {
                    line.red().bold()
                } else if inf.is_mild() {
                    line.yellow()
                } else {
                    line.green()
                };
                writeln!(f, "{coloured}")?;
            }
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
