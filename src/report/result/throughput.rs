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

        // Per-window throughput percentiles. These describe the spread of
        // throughput across fixed wall-clock intervals, complementing the
        // time-averaged mean above. Computed by the same methodology as the
        // average (see `windowed_bps_series`), so `min ≤ avg ≤ max` holds.
        // Only emit them if we actually have successful samples.
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

        let (total_retries, failed_after_retry) = self.retry_statistics();
        if total_retries > 0 {
            writeln!(
                f,
                "  {}: {} ({} failed after retry)",
                "Total Retries".bright_green().bold(),
                total_retries.to_formatted_string(&Locale::en).yellow(),
                failed_after_retry.to_formatted_string(&Locale::en).red()
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

    /// Bucket width (microseconds) for the windowed throughput series.
    ///
    /// Aim for ~100 windows across the measured span, clamped to a 10 ms
    /// floor / 250 ms ceiling. Short tests get proportionally smaller
    /// windows so the percentile spread stays usable; very short tests
    /// collapse to a single window (then min == p50 == max == avg, which
    /// is correct).
    fn analysis_window_us(&self) -> u64 {
        const TARGET_WINDOWS: u64 = 100;
        const MIN_WINDOW_US: u64 = 10_000; // 10 ms
        const MAX_WINDOW_US: u64 = 250_000; // 250 ms
        if self.total_duration_us == 0 {
            return MIN_WINDOW_US;
        }
        (self.total_duration_us / TARGET_WINDOWS).clamp(MIN_WINDOW_US, MAX_WINDOW_US)
    }

    /// Aggregate, mass-conserving per-window throughput series in bits/sec.
    ///
    /// Every non-warmup successful sample's bytes are spread across the
    /// wall-clock windows its `[t_start, t_start + duration)` interval
    /// overlaps, in proportion to the overlap. This is the same
    /// methodology as [`avg_throughput`]: a slow 8 MB HTTP request that
    /// spans seconds is treated as a constant-rate flow over its real
    /// duration rather than as one instantaneous spike, and a UDP send
    /// whose duration is shorter than a window lands entirely in one
    /// window. Because the spread conserves mass
    /// (`Σ window_bytes == bytes_transferred()`) and each window's rate is
    /// `window_bytes * 8 / window_width`, the time-weighted mean of this
    /// series equals `avg_throughput() * 8` exactly — so the percentiles
    /// taken over it always satisfy `min ≤ avg ≤ max`. (The old per-sample
    /// instantaneous rate excluded inter-op gaps — pacing sleeps, request
    /// setup — and so could report a `min` above the wall-clock average.)
    ///
    /// Returns one bits/sec value per window, in time order (unsorted).
    /// Empty when there are no successful samples.
    fn windowed_bps_series(&self, window_us: u64) -> Vec<f64> {
        let window_us = window_us.max(1);
        let span_us = self.total_duration_us;
        if span_us == 0 {
            return Vec::new();
        }

        // Origin = first non-warmup sample start. Sample times include the
        // warmup window; `total_duration_us` excludes it (see
        // `engine::sampler::measurement_duration_us`), so anchor the series
        // to the first measured sample to keep the two axes aligned.
        let origin = self
            .non_warmup_iter()
            .filter(|s| s.is_success())
            .map(|s| s.t_start_us)
            .min();
        let Some(origin) = origin else {
            return Vec::new();
        };

        let n_windows = span_us.div_ceil(window_us) as usize;
        if n_windows == 0 {
            return Vec::new();
        }
        let mut bytes_per_window = vec![0f64; n_windows];

        for s in self.non_warmup_iter().filter(|s| s.is_success()) {
            let bytes = s.bytes as f64;
            if bytes == 0.0 {
                continue;
            }
            // Sample interval on the post-warmup axis, clamped into
            // [0, span). A sample that starts at/after the span end (clock
            // skew at the boundary) or runs past it has its full byte count
            // squeezed into the remaining windows so no mass is lost.
            let raw_start = s.t_start_us.saturating_sub(origin);
            let start = raw_start.min(span_us - 1);
            let raw_end = raw_start.saturating_add(s.duration_us.max(1));
            let end = raw_end.min(span_us).max(start + 1);
            let sample_span = (end - start) as f64;

            let first = (start / window_us) as usize;
            let last = (((end - 1) / window_us) as usize).min(n_windows - 1);
            for (w, bucket) in bytes_per_window
                .iter_mut()
                .enumerate()
                .skip(first)
                .take(last + 1 - first)
            {
                let w_start = (w as u64) * window_us;
                let w_end = (w_start + window_us).min(span_us);
                let overlap = end.min(w_end).saturating_sub(start.max(w_start));
                if overlap > 0 {
                    *bucket += bytes * (overlap as f64 / sample_span);
                }
            }
        }

        (0..n_windows)
            .map(|w| {
                let w_start = (w as u64) * window_us;
                let w_end = (w_start + window_us).min(span_us);
                let width_s = (w_end - w_start) as f64 / 1_000_000.0;
                if width_s <= 0.0 {
                    0.0
                } else {
                    (bytes_per_window[w] * 8.0) / width_s
                }
            })
            .collect()
    }

    /// Percentile of the windowed throughput series, in bits/sec. Consistent
    /// with [`avg_throughput`] (`min ≤ avg ≤ max` always holds); see
    /// [`windowed_bps_series`] for why.
    pub fn percentile_throughput_bps(&self, n: f64) -> Option<f64> {
        if !(0.0..=100.0).contains(&n) {
            return None;
        }
        let mut series = self.windowed_bps_series(self.analysis_window_us());
        if series.is_empty() {
            return None;
        }
        series.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = if n == 0.0 {
            0
        } else if n == 100.0 {
            series.len() - 1
        } else {
            ((n / 100.0) * (series.len() - 1) as f64).round() as usize
        };
        Some(series[idx])
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

    /// Successful application-level requests per second.
    ///
    /// For HTTP every sample is exactly one request (`download_chunk` is one
    /// GET, `upload_chunk` is one POST), so this is the request rate. For
    /// TCP/UDP/QUIC a sample is a read/packet rather than a request, so the
    /// figure is not meaningful — callers gate its display on the protocol.
    pub fn requests_per_second(&self) -> f64 {
        if self.total_duration_us == 0 {
            return 0.0;
        }
        let ok = self.non_warmup_iter().filter(|s| s.is_success()).count();
        (ok as f64) / (self.total_duration_us as f64 / 1_000_000.0)
    }

    /// `(total_retries, failed_after_retry)`. Retries are observable only on
    /// the failure path — `Outcome::Success` carries no retry count — so the
    /// previous "successful after retry" figure was always zero. Reporting it
    /// (and the rate derived from it) showed 0% success on every flaky link,
    /// which was misleading, so it is no longer computed.
    pub fn retry_statistics(&self) -> (u32, u32) {
        let mut total_retries = 0;
        let mut failed_after_retry = 0;
        for s in self.non_warmup_iter() {
            if let Outcome::Failure { retry_count, .. } = &s.outcome {
                total_retries += retry_count;
                failed_after_retry += 1;
            }
        }
        (total_retries, failed_after_retry)
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn result_from(samples: Vec<Sample>, total_duration_us: u64) -> ThroughputResult {
        ThroughputResult {
            streams: vec![StreamSamples {
                stream_id: 0,
                start_offset_us: samples.first().map(|s| s.t_start_us).unwrap_or(0),
                samples,
            }],
            total_duration_us,
            timestamp: Utc::now(),
            udp_stats: None,
            udp_series: Vec::new(),
            udp_series_window_us: 0,
        }
    }

    /// The core invariant the windowing rework guarantees: the windowed
    /// percentiles are consistent with the wall-clock average, so the
    /// physically-impossible `min > avg` the per-sample rate used to
    /// produce can no longer happen.
    fn assert_min_le_avg_le_max(r: &ThroughputResult) {
        let avg_bps = r.avg_throughput() * 8.0;
        let min = r.min_throughput_bps().expect("min");
        let max = r.max_throughput_bps().expect("max");
        // Generous epsilon: the trailing partial window and float rounding
        // can nudge the weighted mean a hair outside the discrete extremes.
        let eps = avg_bps * 1e-6 + 1.0;
        assert!(
            min <= avg_bps + eps,
            "min {min} must not exceed avg {avg_bps}"
        );
        assert!(
            avg_bps <= max + eps,
            "avg {avg_bps} must not exceed max {max}"
        );
        assert!(min <= max + eps, "min {min} must not exceed max {max}");
    }

    #[test]
    fn udp_like_paced_stream_keeps_min_below_avg() {
        // 10_000 tiny datagrams, each "sent" in ~2us but spaced 100us apart
        // by pacing. Per-sample instantaneous rate is ~4.8 Gbps while the
        // paced aggregate is ~96 Mbps — the exact shape that used to make
        // `min` exceed `avg`.
        let mut samples = Vec::new();
        for i in 0..10_000u64 {
            samples.push(Sample::success(i * 100, 2, 1200, false));
        }
        let r = result_from(samples, 1_000_000); // 1s window
        assert_min_le_avg_le_max(&r);
        // Sanity: paced aggregate is ~96 Mbps, nowhere near the per-sample
        // multi-Gbps burst the old code reported as the minimum.
        let max = r.max_throughput_bps().unwrap();
        assert!(max < 500_000_000.0, "windowed max {max} unexpectedly high");
    }

    #[test]
    fn http_like_large_slow_samples_keep_min_le_avg_le_max() {
        // Three back-to-back 8 MB transfers, each spanning 2s. A single
        // sample now spans dozens of windows; spreading its bytes keeps the
        // per-window series flat instead of producing one giant spike.
        let eight_mb = 8 * 1024 * 1024;
        let samples = vec![
            Sample::success(0, 2_000_000, eight_mb, false),
            Sample::success(2_000_000, 2_000_000, eight_mb, false),
            Sample::success(4_000_000, 2_000_000, eight_mb, false),
        ];
        let r = result_from(samples, 6_000_000); // 6s window
        assert_min_le_avg_le_max(&r);
    }

    #[test]
    fn single_window_collapses_to_average() {
        // A test shorter than one window yields a single window where
        // min == p50 == max == avg.
        let r = result_from(vec![Sample::success(0, 500, 4096, false)], 1000);
        let avg_bps = r.avg_throughput() * 8.0;
        let min = r.min_throughput_bps().unwrap();
        let max = r.max_throughput_bps().unwrap();
        assert!((min - max).abs() < 1.0, "single window must be flat");
        assert!((min - avg_bps).abs() <= avg_bps * 1e-6 + 1.0);
    }

    #[test]
    fn no_successful_samples_yields_no_percentile() {
        let r = result_from(Vec::new(), 1_000_000);
        assert!(r.min_throughput_bps().is_none());
        assert!(r.max_throughput_bps().is_none());
    }

    #[test]
    fn requests_per_second_counts_successful_samples_over_duration() {
        // 20 successful requests over a 2s measurement window -> 10 req/s. A
        // warmup sample and a failed sample must not count.
        let mut samples: Vec<Sample> = (0..20).map(|i| Sample::success(i * 1000, 100, 4096, false)).collect();
        samples.push(Sample::success(0, 100, 4096, true)); // warmup, excluded
        samples.push(Sample::failure(
            0,
            100,
            ConnectionError::Timeout("x".into()),
            0,
            false,
        )); // failure, excluded
        let r = result_from(samples, 2_000_000);
        assert!((r.requests_per_second() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn requests_per_second_zero_duration_is_zero() {
        let r = result_from(vec![Sample::success(0, 100, 4096, false)], 0);
        assert_eq!(r.requests_per_second(), 0.0);
    }
}
