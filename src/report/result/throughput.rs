use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Serialize};

use crate::report::{ConnectionError, Outcome, Sample};
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
#[derive(Default)]
pub enum ThroughputAccounting {
    #[default]
    Goodput,
    Wire,
}

/// Per-stream samples for a single parallel connection / stream within
/// a multi-stream throughput test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamSamples {
    /// Index of the stream within the test (0-based).
    pub stream_id: u32,
    /// Wall-clock offset (microseconds from test start) at which this
    /// stream began transferring data. Lets a renderer plot streams
    /// against a common time axis even when they start at different
    /// times.
    pub start_offset_us: u64,
    /// Per-sample observations from this stream.
    pub samples: Vec<Sample>,
}

impl StreamSamples {
    pub fn bytes_transferred(&self) -> u64 {
        self.samples
            .iter()
            .filter(|s| !s.is_warmup && s.is_success())
            .map(|s| s.bytes)
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
    /// Per-stream sample data. The single source of truth for per-sample
    /// observations - aggregate metrics are computed by iterating these
    /// lazily.
    pub streams: Vec<StreamSamples>,
    /// Total measurement duration (post-warmup), in microseconds.
    pub total_duration_us: u64,
    pub timestamp: DateTime<Utc>,

    /// UDP-only: aggregate receiver-side packet stats. Populated locally
    /// for UDP download and from the server REPORT for UDP upload.
    /// `None` for TCP / HTTP runs.
    #[serde(default)]
    pub udp_stats: Option<UdpRunStats>,

    /// UDP-only: per-window receiver-side snapshot series, populated
    /// when the receiver emits periodic stats. Empty (`Vec::new()`) for
    /// TCP / HTTP, or for UDP runs where the series transport failed.
    #[serde(default)]
    pub udp_series: Vec<UdpStatsBucket>,

    /// Width of each `udp_series` bucket, in microseconds. `0` when
    /// `udp_series` is empty.
    #[serde(default)]
    pub udp_series_window_us: u32,
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

/// One window of receiver-side UDP statistics. A vector of these forms a
/// time-series suitable for plotting loss / jitter stability over the
/// whole test duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpStatsBucket {
    /// Offset of the start of this window from the receiver's session
    /// start, in microseconds. The receiver and sender share the same
    /// epoch (negotiated in the `Hello` handshake) so this aligns with
    /// `Sample.t_start_us` on the corresponding stream.
    pub t_offset_us: u64,
    pub received: u64,
    pub bytes_received: u64,
    pub lost: u64,
    pub out_of_order: u64,
    pub duplicates: u64,
    /// RFC 3550 jitter at the end of the window, in microseconds.
    pub jitter_us: u32,
}

impl ThroughputResult {
    /// Iterator over every sample across every stream. Aggregate metrics
    /// build on this; warmup-aware variants below filter out
    /// `is_warmup` samples.
    pub fn samples_iter(&self) -> impl Iterator<Item = &Sample> {
        self.streams.iter().flat_map(|s| s.samples.iter())
    }

    /// Iterator over non-warmup samples — what aggregate stats use.
    pub fn non_warmup_iter(&self) -> impl Iterator<Item = &Sample> {
        self.samples_iter().filter(|s| !s.is_warmup)
    }

    /// Total non-warmup duration as a `Duration`.
    pub fn total_duration(&self) -> Duration {
        Duration::from_micros(self.total_duration_us)
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
            format!("{:.2}s", self.total_duration().as_secs_f64()).yellow()
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
                    format_size(bps as u64, DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"),)
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
            "Samples".bright_green().bold(),
            self.sample_count().to_formatted_string(&Locale::en).white()
        )?;

        // Per-stream breakdown, only when we have more than one stream.
        // For single-stream tests the aggregate above is everything you need.
        if self.streams.len() > 1 {
            writeln!(f, "  {}:", "Per-Stream".bright_green().bold())?;
            for s in &self.streams {
                let bps = s.avg_throughput_bps(self.total_duration());
                writeln!(
                    f,
                    "    stream {:>3}: {} ({} samples)",
                    s.stream_id.to_string().yellow(),
                    format_size(bps as u64, DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"))
                        .magenta(),
                    s.samples.len().to_formatted_string(&Locale::en).white()
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

        if !self.udp_series.is_empty() {
            writeln!(
                f,
                "  {}: {} buckets @ {} ms each",
                "UDP Stats Series".bright_green().bold(),
                self.udp_series
                    .len()
                    .to_formatted_string(&Locale::en)
                    .cyan(),
                (self.udp_series_window_us / 1000)
                    .to_formatted_string(&Locale::en)
                    .yellow()
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
    /// Total non-warmup samples across all streams (success + failure).
    pub fn sample_count(&self) -> usize {
        self.non_warmup_iter().count()
    }

    /// Total bytes transferred across non-warmup successful samples.
    pub fn bytes_transferred(&self) -> u64 {
        self.non_warmup_iter()
            .filter(|s| s.is_success())
            .map(|s| s.bytes)
            .sum()
    }

    /// Average throughput in bytes per second.
    pub fn avg_throughput(&self) -> f64 {
        if self.total_duration_us == 0 {
            return 0.0;
        }
        (self.bytes_transferred() as f64) / (self.total_duration_us as f64 / 1_000_000.0)
    }

    /// Estimate wire-rate average throughput in bits per second by adding
    /// per-segment / per-packet framing overhead to the goodput numbers.
    pub fn avg_throughput_wire_bps(&self, overhead_per_segment: usize, mtu: usize) -> f64 {
        if self.total_duration_us == 0 || mtu == 0 {
            return 0.0;
        }
        let payload_per_segment = mtu.saturating_sub(overhead_per_segment).max(1) as u64;
        let total_overhead: u64 = self
            .non_warmup_iter()
            .filter(|s| s.is_success())
            .map(|s| {
                let segments = s.bytes.div_ceil(payload_per_segment);
                segments * overhead_per_segment as u64
            })
            .sum();
        let total_wire_bytes = self.bytes_transferred() + total_overhead;
        (total_wire_bytes as f64 * 8.0) / (self.total_duration_us as f64 / 1_000_000.0)
    }

    /// Per-sample throughput in bps for non-warmup successful samples,
    /// sorted ascending. Used by percentile helpers.
    fn sample_bps_sorted(&self) -> Vec<f64> {
        let mut samples: Vec<f64> = self
            .non_warmup_iter()
            .filter(|s| s.is_success())
            .map(|s| s.throughput_bps())
            .collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        samples
    }

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

    pub fn min_throughput_bps(&self) -> Option<f64> {
        self.percentile_throughput_bps(0.0)
    }

    pub fn max_throughput_bps(&self) -> Option<f64> {
        self.percentile_throughput_bps(100.0)
    }

    pub fn connection_success_rate(&self) -> f64 {
        let total = self.non_warmup_iter().count();
        if total == 0 {
            return 0.0;
        }
        let successful = self.non_warmup_iter().filter(|s| s.is_success()).count();
        successful as f64 / total as f64
    }

    pub fn request_success_rate(&self) -> f64 {
        self.connection_success_rate()
    }

    /// `(total_retries, successful_after_retry, failed_after_retry)`.
    pub fn retry_statistics(&self) -> (u32, u32, u32) {
        let mut total_retries = 0;
        let successful_after_retry = 0;
        let mut failed_after_retry = 0;
        for s in self.non_warmup_iter() {
            if let Outcome::Failure { retry_count, .. } = &s.outcome {
                total_retries += retry_count;
                failed_after_retry += 1;
            }
        }
        (total_retries, successful_after_retry, failed_after_retry)
    }

    pub fn retry_success_rate(&self) -> f64 {
        let (total_retries, successful_after_retry, failed_after_retry) = self.retry_statistics();
        if total_retries == 0 {
            return 1.0;
        }
        successful_after_retry as f64 / (successful_after_retry + failed_after_retry) as f64
    }

    pub fn error_distribution(&self) -> HashMap<String, u32> {
        let mut distribution = HashMap::new();
        for s in self.non_warmup_iter() {
            if let Outcome::Failure { error, .. } = &s.outcome {
                let kind = match error {
                    ConnectionError::ConnectionFailed(_) => "Connection Failed",
                    ConnectionError::TransferFailed(_) => "Transfer Failed",
                    ConnectionError::Timeout(_) => "Timeout",
                    ConnectionError::Unknown(_) => "Unknown",
                };
                *distribution.entry(kind.to_string()).or_insert(0) += 1;
            }
        }
        distribution
    }

    pub fn total_errors(&self) -> u32 {
        self.non_warmup_iter().filter(|s| !s.is_success()).count() as u32
    }
}
