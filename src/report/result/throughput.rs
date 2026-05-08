use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Serialize};

use crate::report::{ConnectionError, ThroughputMeasurement};
use std::collections::HashMap;
use std::fmt;

/// Per-segment / per-packet framing overhead used when extrapolating
/// goodput measurements to wire-rate. These match common IPv4 settings:
/// 20 B IP + 20 B TCP + 12 B TCP timestamp options = 52 B/segment, and
/// 20 B IP + 8 B UDP = 28 B/packet. For IPv6 the IP header is 20 bytes
/// larger; we keep IPv4 as the default since most measurement
/// environments still use it.
pub const WIRE_OVERHEAD_TCP_BYTES: usize = 52;
pub const WIRE_OVERHEAD_UDP_BYTES: usize = 28;
/// Standard Ethernet MTU. Used to estimate segment count from payload size.
pub const STANDARD_MTU: usize = 1500;

/// Choose how throughput is reported.
///
/// `Goodput` (the default) counts only application-layer payload bytes;
/// this is the behavior speed-cli has always had and what most users mean
/// when comparing protocols. `Wire` adds an estimate of per-segment /
/// per-packet framing overhead (TCP/IP or UDP/IP), giving a number closer
/// to what you would see on a NIC. The wire estimate is only as accurate
/// as the assumed MTU and header sizes - documented above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThroughputAccounting {
    Goodput,
    Wire,
}

impl Default for ThroughputAccounting {
    fn default() -> Self {
        ThroughputAccounting::Goodput
    }
}

/// Per-stream measurements for a single parallel connection / stream
/// within a multi-stream throughput test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamMeasurements {
    /// Index of the stream within the test (0-based).
    pub stream_id: usize,
    /// Per-chunk measurements from this stream.
    pub measurements: Vec<ThroughputMeasurement>,
}

impl StreamMeasurements {
    pub fn bytes_transferred(&self) -> u64 {
        self.measurements
            .iter()
            .map(|m| match m {
                ThroughputMeasurement::Success { bytes, .. } => *bytes,
                ThroughputMeasurement::Failure { .. } => 0,
            })
            .sum()
    }

    /// Average throughput for this stream over the given window, in
    /// bits per second.
    pub fn avg_throughput_bps(&self, window: Duration) -> f64 {
        if window.is_zero() {
            return 0.0;
        }
        (self.bytes_transferred() as f64 * 8.0) / window.as_secs_f64()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputResult {
    /// Aggregate measurements (flatten of all streams). Used for the
    /// summary metrics and kept stable for older consumers.
    pub measurements: Vec<ThroughputMeasurement>,
    /// Per-stream breakdown. Empty for tests that don't have a notion of
    /// streams (e.g., today's UDP path or imported reports written
    /// before per-stream data was tracked).
    #[serde(default)]
    pub streams: Vec<StreamMeasurements>,
    /// Total duration of the test
    pub total_duration: Duration,

    pub timestamp: DateTime<Utc>,

    /// UDP-only: receiver-side packet stats. Populated locally for UDP
    /// download (the client receives) and from the server REPORT for
    /// UDP upload (the server receives). `None` for TCP / HTTP runs.
    #[serde(default)]
    pub udp_stats: Option<UdpRunStats>,
}

/// Receiver-side UDP packet accounting. Shipped per direction in
/// [`ThroughputResult::udp_stats`]. Whichever side acted as receiver
/// for the run is the side these stats describe — the field
/// [`UdpRunStats::observed_by`] makes that explicit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpRunStats {
    pub observed_by: UdpStatsSide,
    pub received_packets: u64,
    pub bytes_received: u64,
    pub lost_packets: u64,
    pub out_of_order: u64,
    pub duplicates: u64,
    /// RFC 3550 interarrival jitter in microseconds.
    pub jitter_us: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UdpStatsSide {
    /// Stats computed by the local (client) side. Used for downloads
    /// where the client is the receiver.
    Local,
    /// Stats reported by the remote (server) side via the blaster
    /// protocol's REPORT packet. Used for uploads.
    Remote,
}

impl UdpRunStats {
    /// Loss as a fraction in [0.0, 1.0]. Returns `None` if no packets
    /// were sent (avoids 0/0).
    pub fn loss_fraction(&self) -> Option<f64> {
        let sent = self.received_packets + self.lost_packets;
        if sent == 0 {
            return None;
        }
        Some(self.lost_packets as f64 / sent as f64)
    }
}

impl ThroughputResult {
    /// Reduce per-measurement detail to at most `max_samples` entries
    /// per stream (and per aggregate). At multi-Gbps speeds the
    /// untrimmed vector grows into the hundreds of thousands and pushes
    /// JSON exports past 100 MB; this caps the file size while
    /// preserving percentile fidelity (uniform-stride decimation, which
    /// keeps the order statistics).
    ///
    /// Aggregate stats already computed against the full vector are
    /// unaffected; this only mutates the on-disk representation.
    pub fn downsample_for_export(&mut self, max_samples: usize) {
        fn decimate<T: Clone>(v: &mut Vec<T>, max: usize) {
            if v.len() <= max || max == 0 {
                return;
            }
            let stride = v.len() as f64 / max as f64;
            let mut out = Vec::with_capacity(max);
            let mut i = 0.0;
            while (i as usize) < v.len() && out.len() < max {
                out.push(v[i as usize].clone());
                i += stride;
            }
            *v = out;
        }
        decimate(&mut self.measurements, max_samples);
        for s in &mut self.streams {
            decimate(&mut s.measurements, max_samples);
        }
    }
}

impl fmt::Display for ThroughputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Data Transferred".bright_green().bold(),
            format_size(self.bytes_transferred(), BINARY).cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_green().bold(),
            format!("{:.2}s", self.total_duration.as_secs_f64()).yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Average Throughput".bright_green().bold(),
            format_size(
                (self.avg_throughput() * 8.0) as u64,
                DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"),
            )
            .magenta()
        )?;

        // Per-sample throughput percentiles. These describe the spread of the
        // individual transfer measurements, complementing the time-averaged
        // mean above. Only emit them if we actually have successful samples.
        let percentiles = [
            ("Min Throughput", 0.0),
            ("p50 Throughput", 50.0),
            ("p90 Throughput", 90.0),
            ("p95 Throughput", 95.0),
            ("p99 Throughput", 99.0),
            ("Max Throughput", 100.0),
        ];
        for (label, p) in percentiles {
            if let Some(bps) = self.percentile_throughput_bps(p) {
                writeln!(
                    f,
                    "  {}: {}",
                    label.bright_green().bold(),
                    format_size(
                        bps as u64,
                        DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"),
                    )
                    .magenta()
                )?;
            }
        }
        writeln!(
            f,
            "  {}: {}",
            "Connection Success Rate".bright_green().bold(),
            format!("{:.1}%", self.connection_success_rate() * 100.0).green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Request Success Rate".bright_green().bold(),
            format!("{:.1}%", self.request_success_rate() * 100.0).green()
        )?;

        let (total_retries, successful_after_retry, failed_after_retry) = self.retry_statistics();
        if total_retries > 0 {
            writeln!(
                f,
                "  {}: {} (Success: {}, Failed: {})",
                "Total Retries".bright_green().bold(),
                total_retries.to_formatted_string(&Locale::en).yellow(),
                successful_after_retry
                    .to_formatted_string(&Locale::en)
                    .green(),
                failed_after_retry.to_formatted_string(&Locale::en).red()
            )?;

            writeln!(
                f,
                "  {}: {:.1}%",
                "Retry Success Rate".bright_green().bold(),
                self.retry_success_rate() * 100.0
            )?;
        }

        let error_distribution = self.error_distribution();
        if !error_distribution.is_empty() {
            writeln!(
                f,
                "  {}: {} total",
                "Errors".bright_green().bold(),
                self.total_errors().to_formatted_string(&Locale::en).red()
            )?;
            for (error_type, count) in error_distribution {
                writeln!(
                    f,
                    "    {}: {}",
                    error_type.bright_yellow(),
                    count.to_formatted_string(&Locale::en).red()
                )?;
            }
        }

        writeln!(
            f,
            "  {}: {}",
            "Measurements".bright_green().bold(),
            self.measurements
                .len()
                .to_formatted_string(&Locale::en)
                .white()
        )?;

        // Per-stream breakdown, only when we have more than one stream.
        // For single-stream tests the aggregate above is everything you need.
        if self.streams.len() > 1 {
            writeln!(
                f,
                "  {}:",
                "Per-Stream".bright_green().bold()
            )?;
            for s in &self.streams {
                let bps = s.avg_throughput_bps(self.total_duration);
                writeln!(
                    f,
                    "    stream {:>3}: {} ({} chunks)",
                    s.stream_id.to_string().yellow(),
                    format_size(
                        bps as u64,
                        DECIMAL.base_unit(BaseUnit::Bit).suffix("/s")
                    )
                    .magenta(),
                    s.measurements.len().to_formatted_string(&Locale::en).white()
                )?;
            }
        }

        if let Some(udp) = &self.udp_stats {
            let side = match udp.observed_by {
                UdpStatsSide::Local => "client-local",
                UdpStatsSide::Remote => "server-reported",
            };
            writeln!(
                f,
                "  {} ({}):",
                "UDP Packet Stats".bright_green().bold(),
                side.bright_blue()
            )?;
            writeln!(
                f,
                "    Packets received: {}",
                udp.received_packets.to_formatted_string(&Locale::en).cyan()
            )?;
            writeln!(
                f,
                "    Lost: {} {}",
                udp.lost_packets.to_formatted_string(&Locale::en).red(),
                match udp.loss_fraction() {
                    Some(f) => format!("({:.3}%)", f * 100.0),
                    None => String::new(),
                }
                .red()
            )?;
            writeln!(
                f,
                "    Out-of-order: {}",
                udp.out_of_order.to_formatted_string(&Locale::en).yellow()
            )?;
            writeln!(
                f,
                "    Duplicates: {}",
                udp.duplicates.to_formatted_string(&Locale::en).yellow()
            )?;
            writeln!(
                f,
                "    Jitter (RFC 3550): {} us",
                udp.jitter_us.to_formatted_string(&Locale::en).magenta()
            )?;
        }

        writeln!(
            f,
            "  {}: {}",
            "Timestamp".bright_green().bold(),
            self.timestamp
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .blue()
        )?;

        Ok(())
    }
}

impl ThroughputResult {
    /// Returns total number of bytes transferred
    pub fn bytes_transferred(&self) -> u64 {
        self.measurements
            .iter()
            .map(|m| match m {
                ThroughputMeasurement::Success { bytes, .. } => *bytes,
                ThroughputMeasurement::Failure { .. } => 0,
            })
            .sum()
    }

    /// Returns the average throughput in bytes per second
    pub fn avg_throughput(&self) -> f64 {
        if self.total_duration.is_zero() {
            return 0.0;
        }

        (self.bytes_transferred() as f64) / self.total_duration.as_secs_f64()
    }

    /// Estimate wire-rate average throughput in bits per second by adding
    /// per-segment / per-packet framing overhead to the goodput numbers.
    ///
    /// `overhead_per_segment` should be one of `WIRE_OVERHEAD_TCP_BYTES`
    /// or `WIRE_OVERHEAD_UDP_BYTES` (for IPv4); pass `mtu = STANDARD_MTU`
    /// unless you've measured otherwise.
    pub fn avg_throughput_wire_bps(
        &self,
        overhead_per_segment: usize,
        mtu: usize,
    ) -> f64 {
        if self.total_duration.is_zero() || mtu == 0 {
            return 0.0;
        }
        let payload_per_segment = mtu.saturating_sub(overhead_per_segment).max(1) as u64;
        let total_overhead: u64 = self
            .measurements
            .iter()
            .filter_map(|m| match m {
                ThroughputMeasurement::Success { bytes, .. } => Some(*bytes),
                ThroughputMeasurement::Failure { .. } => None,
            })
            .map(|bytes| {
                let segments = bytes.div_ceil(payload_per_segment);
                segments * overhead_per_segment as u64
            })
            .sum();
        let total_wire_bytes = self.bytes_transferred() + total_overhead;
        (total_wire_bytes as f64 * 8.0) / self.total_duration.as_secs_f64()
    }

    /// Returns per-measurement throughput samples in bits per second, for
    /// successful measurements only.
    fn sample_bps_sorted(&self) -> Vec<f64> {
        let mut samples: Vec<f64> = self
            .measurements
            .iter()
            .filter_map(|m| match m {
                ThroughputMeasurement::Success { .. } => Some(m.throughput_bps()),
                ThroughputMeasurement::Failure { .. } => None,
            })
            .collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        samples
    }

    /// Returns the n-th percentile of per-measurement throughput in bits per
    /// second. Returns None if there are no successful samples or `n` is
    /// outside [0, 100].
    pub fn percentile_throughput_bps(&self, n: f64) -> Option<f64> {
        if !(0.0..=100.0).contains(&n) {
            return None;
        }
        let samples = self.sample_bps_sorted();
        if samples.is_empty() {
            return None;
        }
        if n == 0.0 {
            return Some(samples[0]);
        }
        if n == 100.0 {
            return Some(samples[samples.len() - 1]);
        }
        let index = ((n / 100.0) * (samples.len() - 1) as f64).round() as usize;
        Some(samples[index])
    }

    /// Min per-measurement throughput in bits per second.
    pub fn min_throughput_bps(&self) -> Option<f64> {
        self.percentile_throughput_bps(0.0)
    }

    /// Max per-measurement throughput in bits per second.
    pub fn max_throughput_bps(&self) -> Option<f64> {
        self.percentile_throughput_bps(100.0)
    }

    /// Returns the connection success rate as a percentage (0.0 to 1.0)
    pub fn connection_success_rate(&self) -> f64 {
        if self.measurements.is_empty() {
            return 0.0;
        }

        let successful_connections = self
            .measurements
            .iter()
            .filter(|m| matches!(m, ThroughputMeasurement::Success { .. }))
            .count();

        successful_connections as f64 / self.measurements.len() as f64
    }

    /// Returns the request success rate as a percentage (0.0 to 1.0)
    /// This is the same as connection success rate in this context
    pub fn request_success_rate(&self) -> f64 {
        self.connection_success_rate()
    }

    /// Returns retry statistics: (total_retries, successful_after_retry, failed_after_retry)
    pub fn retry_statistics(&self) -> (u32, u32, u32) {
        let mut total_retries = 0;
        let successful_after_retry = 0;
        let mut failed_after_retry = 0;

        for measurement in &self.measurements {
            match measurement {
                ThroughputMeasurement::Success { .. } => {
                    // For successful measurements, we assume no retries were needed
                    // This could be enhanced if success measurements tracked retry count
                }
                ThroughputMeasurement::Failure { retry_count, .. } => {
                    total_retries += retry_count;
                    failed_after_retry += 1;
                }
            }
        }

        (total_retries, successful_after_retry, failed_after_retry)
    }

    /// Returns the success rate after retries (0.0 to 1.0)
    pub fn retry_success_rate(&self) -> f64 {
        let (total_retries, successful_after_retry, failed_after_retry) = self.retry_statistics();

        if total_retries == 0 {
            return 1.0; // No retries needed means 100% success
        }

        successful_after_retry as f64 / (successful_after_retry + failed_after_retry) as f64
    }

    /// Returns error distribution by type
    pub fn error_distribution(&self) -> HashMap<String, u32> {
        let mut distribution = HashMap::new();

        for measurement in &self.measurements {
            if let ThroughputMeasurement::Failure { error, .. } = measurement {
                let error_type = match error {
                    ConnectionError::ConnectionFailed(_) => "Connection Failed",
                    ConnectionError::TransferFailed(_) => "Transfer Failed",
                    ConnectionError::Timeout(_) => "Timeout",
                    ConnectionError::Unknown(_) => "Unknown",
                };

                *distribution.entry(error_type.to_string()).or_insert(0) += 1;
            }
        }

        distribution
    }

    /// Returns the total number of errors
    pub fn total_errors(&self) -> u32 {
        self.measurements
            .iter()
            .filter(|m| matches!(m, ThroughputMeasurement::Failure { .. }))
            .count() as u32
    }
}
