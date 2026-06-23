use std::fmt::{self, Display, Formatter};

use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};

use crate::report::LatencyMeasurement;
use crate::utils::sparkline::latency_sparkline;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    /// List of RTT measurements (in microseconds, as `Option<u64>`).
    pub measurements: Vec<LatencyMeasurement>,
    pub timestamp: DateTime<Utc>,
}

impl LatencyResult {
    /// All non-dropped RTTs in milliseconds (the unit the renderer and
    /// Display impls operate in).
    pub fn rtts_ms(&self) -> Vec<f64> {
        self.measurements
            .iter()
            .filter_map(|m| m.rtt_ms())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.measurements.len()
    }

    pub fn successful_count(&self) -> usize {
        self.measurements
            .iter()
            .filter(|m| m.rtt_us.is_some())
            .count()
    }

    pub fn dropped_count(&self) -> usize {
        self.measurements
            .iter()
            .filter(|m| m.rtt_us.is_none())
            .count()
    }

    pub fn avg_rtt(&self) -> Option<f64> {
        let rtts = self.rtts_ms();
        if rtts.is_empty() {
            return None;
        }
        Some(rtts.iter().sum::<f64>() / rtts.len() as f64)
    }

    pub fn min_rtt(&self) -> Option<f64> {
        self.rtts_ms()
            .into_iter()
            .fold(None, |acc, rtt| Some(acc.map_or(rtt, |m| rtt.min(m))))
    }

    pub fn percentile_rtt(&self, n: f64) -> Option<f64> {
        if !(0.0..=100.0).contains(&n) {
            return None;
        }
        let mut rtts = self.rtts_ms();
        if rtts.is_empty() {
            return None;
        }
        rtts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if n == 0.0 {
            return Some(rtts[0]);
        }
        if n == 100.0 {
            return Some(rtts[rtts.len() - 1]);
        }
        let index = ((n / 100.0) * (rtts.len() - 1) as f64).round() as usize;
        Some(rtts[index])
    }

    pub fn max_rtt(&self) -> Option<f64> {
        self.rtts_ms()
            .into_iter()
            .fold(None, |acc, rtt| Some(acc.map_or(rtt, |m| rtt.max(m))))
    }

    /// 90th-percentile RTT in milliseconds. The tail is where WiFi / AP
    /// latency spikes hide: the median can look healthy while the tail tells
    /// the real story of a contended link.
    pub fn p90_rtt(&self) -> Option<f64> {
        self.percentile_rtt(90.0)
    }

    /// 95th-percentile RTT in milliseconds.
    pub fn p95_rtt(&self) -> Option<f64> {
        self.percentile_rtt(95.0)
    }

    /// 99th-percentile RTT in milliseconds.
    pub fn p99_rtt(&self) -> Option<f64> {
        self.percentile_rtt(99.0)
    }

    /// 99.9th-percentile RTT in milliseconds.
    pub fn p999_rtt(&self) -> Option<f64> {
        self.percentile_rtt(99.9)
    }

    /// Detect latency spikes across the time-series. Returns `None` when there
    /// are no successful samples to establish a baseline from.
    ///
    /// A *spike* is a successful probe whose RTT exceeds an adaptive threshold
    /// derived from the median: `max(median * 3, median + 20 ms)`. The
    /// multiplicative term catches proportional blow-ups on an already-slow
    /// link; the additive floor stops a tiny sub-millisecond median from
    /// flagging every minor wobble. Spikes are the fingerprint of a contended
    /// WiFi link or AP — bufferbloat, airtime contention, power-save wakeups,
    /// background scans, rate adaptation.
    ///
    /// Everything here is derived on demand from `measurements`; nothing is
    /// serialized.
    pub fn spike_report(&self) -> Option<LatencySpikeReport> {
        const SPIKE_FACTOR: f64 = 3.0;
        const SPIKE_FLOOR_MS: f64 = 20.0;
        const MAX_SPIKE_OFFSETS: usize = 10;

        let baseline_ms = self.percentile_rtt(50.0)?;
        let successful = self.successful_count();
        // `percentile_rtt` only returns `Some` when there is at least one
        // successful sample, so `successful` is guaranteed non-zero here.
        let threshold_ms = (baseline_ms * SPIKE_FACTOR).max(baseline_ms + SPIKE_FLOOR_MS);

        let mut spike_count = 0usize;
        let mut worst_ms = baseline_ms;
        let mut spike_offsets_ms = Vec::new();
        for m in &self.measurements {
            let Some(rtt) = m.rtt_ms() else { continue };
            if rtt > worst_ms {
                worst_ms = rtt;
            }
            if rtt > threshold_ms {
                spike_count += 1;
                if spike_offsets_ms.len() < MAX_SPIKE_OFFSETS {
                    spike_offsets_ms.push(m.t_start_us as f64 / 1000.0);
                }
            }
        }

        let spike_rate_pct = (spike_count as f64 / successful as f64) * 100.0;
        let verdict = SpikeVerdict::classify(spike_count, spike_rate_pct, worst_ms, baseline_ms);

        Some(LatencySpikeReport {
            baseline_ms,
            threshold_ms,
            spike_count,
            spike_rate_pct,
            worst_ms,
            spike_offsets_ms,
            verdict,
        })
    }

    pub fn rtt_stddev(&self) -> Option<f64> {
        let rtts = self.rtts_ms();
        if rtts.is_empty() {
            return None;
        }
        let mean = self.avg_rtt()?;
        let variance =
            rtts.iter().map(|&rtt| (rtt - mean).powi(2)).sum::<f64>() / rtts.len() as f64;
        Some(variance.sqrt())
    }

    /// RFC 3550 jitter in milliseconds.
    ///
    /// ```text
    /// J(0) = 0
    /// J(i) = J(i-1) + (|D(i-1, i)| - J(i-1)) / 16
    /// ```
    /// where `D(i-1, i)` is the difference between the inter-sample arrival
    /// gap and the corresponding RTT delta. Captures sustained timing
    /// variation rather than overall RTT spread. Returns None if there are
    /// fewer than two successful samples.
    pub fn jitter_rfc3550(&self) -> Option<f64> {
        let mut prev: Option<(f64, f64)> = None; // (rtt_ms, t_start_ms)
        let mut jitter: f64 = 0.0;
        let mut updates: u32 = 0;

        for m in &self.measurements {
            let Some(rtt) = m.rtt_ms() else { continue };
            let elapsed_ms = m.t_start_us as f64 / 1000.0;
            if let Some((prev_rtt, prev_elapsed)) = prev {
                let arrival_gap = elapsed_ms - prev_elapsed;
                let rtt_delta = rtt - prev_rtt;
                let d = (arrival_gap - rtt_delta).abs();
                jitter += (d - jitter) / 16.0;
                updates += 1;
            }
            prev = Some((rtt, elapsed_ms));
        }

        if updates == 0 { None } else { Some(jitter) }
    }
}

/// How spiky a latency series is. Spikes are the signature of a congested or
/// contended WiFi link / AP. Derived by [`LatencyResult::spike_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpikeVerdict {
    /// No probe crossed the spike threshold.
    Clean,
    /// A small number of mild spikes — usually tolerable.
    Occasional,
    /// Spikes are frequent, or at least one is severe (an order of magnitude
    /// over the steady-state median). The link is misbehaving.
    Frequent,
}

impl SpikeVerdict {
    fn classify(spike_count: usize, spike_rate_pct: f64, worst_ms: f64, baseline_ms: f64) -> Self {
        if spike_count == 0 {
            return SpikeVerdict::Clean;
        }
        // Frequent if spikes are common (≥2% of probes) or any single spike is
        // an order of magnitude over the steady-state median.
        if spike_rate_pct >= 2.0 || worst_ms >= baseline_ms * 10.0 {
            SpikeVerdict::Frequent
        } else {
            SpikeVerdict::Occasional
        }
    }

    /// Short human label for the verdict.
    pub fn label(&self) -> &'static str {
        match self {
            SpikeVerdict::Clean => "Clean",
            SpikeVerdict::Occasional => "Occasional latency spikes",
            SpikeVerdict::Frequent => "Frequent latency spikes",
        }
    }
}

/// Spike-detection summary for a latency series. Computed on demand from the
/// raw measurements; not serialized.
#[derive(Debug, Clone)]
pub struct LatencySpikeReport {
    /// Steady-state reference (median RTT), in milliseconds.
    pub baseline_ms: f64,
    /// Adaptive threshold above which a probe counts as a spike, in ms.
    pub threshold_ms: f64,
    /// Number of successful probes above `threshold_ms`.
    pub spike_count: usize,
    /// Spikes as a percentage of successful probes.
    pub spike_rate_pct: f64,
    /// Worst (maximum) RTT observed, in milliseconds.
    pub worst_ms: f64,
    /// Test-timeline offsets (ms) of the first few spikes, for the report.
    pub spike_offsets_ms: Vec<f64>,
    pub verdict: SpikeVerdict,
}

impl Display for LatencySpikeReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.verdict {
            SpikeVerdict::Clean => write!(
                f,
                "{} (no probe over {:.1} ms)",
                self.verdict.label(),
                self.threshold_ms
            ),
            _ => write!(
                f,
                "{} ({} over {:.1} ms, {:.1}% of probes, worst {:.1} ms vs {:.1} ms baseline)",
                self.verdict.label(),
                self.spike_count,
                self.threshold_ms,
                self.spike_rate_pct,
                self.worst_ms,
                self.baseline_ms
            ),
        }
    }
}

impl Display for LatencyResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let total_count = self.count();
        let successful_count = self.successful_count();
        let dropped_count = self.dropped_count();
        let loss_rate = if total_count > 0 {
            (dropped_count as f64 / total_count as f64) * 100.0
        } else {
            0.0
        };

        writeln!(
            f,
            "    {}: {}",
            "Total Measurements".bright_blue().bold(),
            total_count.to_string().white()
        )?;
        writeln!(
            f,
            "    {}: {}",
            "Successful".bright_blue().bold(),
            successful_count.to_string().green()
        )?;
        writeln!(
            f,
            "    {}: {}",
            "Dropped".bright_blue().bold(),
            dropped_count.to_string().red()
        )?;
        writeln!(
            f,
            "    {}: {}",
            "Packet Loss".bright_blue().bold(),
            format!("{loss_rate:.2}%").red()
        )?;

        if let Some(avg) = self.avg_rtt() {
            writeln!(
                f,
                "    {}: {}",
                "Average RTT".bright_blue().bold(),
                format!("{avg:.2} ms").cyan()
            )?;
        }
        if let Some(min) = self.min_rtt() {
            writeln!(
                f,
                "    {}: {}",
                "Min RTT".bright_blue().bold(),
                format!("{min:.2} ms").green()
            )?;
        }
        if let Some(p25) = self.percentile_rtt(25.0) {
            writeln!(
                f,
                "    {}: {}",
                "25th Percentile RTT".bright_blue().bold(),
                format!("{p25:.2} ms").yellow()
            )?;
        }
        if let Some(p50) = self.percentile_rtt(50.0) {
            writeln!(
                f,
                "    {}: {}",
                "Median RTT".bright_blue().bold(),
                format!("{p50:.2} ms").yellow()
            )?;
        }
        if let Some(p75) = self.percentile_rtt(75.0) {
            writeln!(
                f,
                "    {}: {}",
                "75th Percentile RTT".bright_blue().bold(),
                format!("{p75:.2} ms").yellow()
            )?;
        }
        if let Some(max) = self.max_rtt() {
            writeln!(
                f,
                "    {}: {}",
                "Max RTT".bright_blue().bold(),
                format!("{max:.2} ms").yellow()
            )?;
        }
        if let Some(stddev) = self.rtt_stddev() {
            writeln!(
                f,
                "    {}: {}",
                "RTT Stddev".bright_blue().bold(),
                format!("{stddev:.2} ms").magenta()
            )?;
        }
        if let Some(jitter) = self.jitter_rfc3550() {
            writeln!(
                f,
                "    {}: {}",
                "Jitter (RFC 3550)".bright_blue().bold(),
                format!("{jitter:.2} ms").magenta()
            )?;
        }

        // Tail percentiles — where WiFi / AP latency spikes show up.
        if let (Some(p95), Some(p99)) = (self.p95_rtt(), self.p99_rtt()) {
            let p999 = self.p999_rtt().unwrap_or(p99);
            writeln!(
                f,
                "    {}: {}",
                "Tail RTT (p95/p99/p99.9)".bright_blue().bold(),
                format!("{p95:.2} / {p99:.2} / {p999:.2} ms").yellow()
            )?;
        }

        // Spike verdict, coloured by severity.
        if let Some(sr) = self.spike_report() {
            let line = sr.to_string();
            let coloured = match sr.verdict {
                SpikeVerdict::Clean => line.green(),
                SpikeVerdict::Occasional => line.yellow(),
                SpikeVerdict::Frequent => line.red().bold(),
            };
            writeln!(f, "    {}: {}", "Spikes".bright_blue().bold(), coloured)?;
        }

        // Time-vs-latency chart so spikes are visible at a glance.
        let chart = latency_sparkline(&self.measurements, 60, 5);
        if !chart.is_empty() {
            writeln!(f, "    {}:", "Latency over time".bright_blue().bold())?;
            for line in chart.lines() {
                writeln!(f, "    {line}")?;
            }
        }

        writeln!(
            f,
            "    {}: {}",
            "Timestamp".bright_blue().bold(),
            self.timestamp
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .blue()
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_result(rtts_ms: Vec<Option<f64>>) -> LatencyResult {
        let measurements = rtts_ms
            .into_iter()
            .map(|rtt_ms| LatencyMeasurement {
                t_start_us: 0,
                rtt_us: rtt_ms.map(|ms| (ms * 1000.0) as u64),
            })
            .collect();
        LatencyResult {
            measurements,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_percentile_calculation() {
        let result = build_result(vec![
            Some(10.0),
            Some(20.0),
            Some(30.0),
            Some(40.0),
            Some(50.0),
        ]);
        assert_eq!(result.percentile_rtt(0.0), Some(10.0));
        assert_eq!(result.percentile_rtt(25.0), Some(20.0));
        assert_eq!(result.percentile_rtt(50.0), Some(30.0));
        assert_eq!(result.percentile_rtt(75.0), Some(40.0));
        assert_eq!(result.percentile_rtt(100.0), Some(50.0));
    }

    #[test]
    fn test_percentile_with_unsorted_values() {
        let result = build_result(vec![
            Some(50.0),
            Some(10.0),
            Some(30.0),
            Some(20.0),
            Some(40.0),
        ]);
        assert_eq!(result.percentile_rtt(0.0), Some(10.0));
        assert_eq!(result.percentile_rtt(50.0), Some(30.0));
        assert_eq!(result.percentile_rtt(100.0), Some(50.0));
    }

    #[test]
    fn test_percentile_with_dropped_measurements() {
        let result = build_result(vec![Some(10.0), None, Some(30.0), None, Some(50.0)]);
        assert_eq!(result.percentile_rtt(0.0), Some(10.0));
        assert_eq!(result.percentile_rtt(50.0), Some(30.0));
        assert_eq!(result.percentile_rtt(100.0), Some(50.0));
    }

    #[test]
    fn test_percentile_invalid_range() {
        let result = build_result(vec![Some(10.0), Some(20.0)]);
        assert_eq!(result.percentile_rtt(-1.0), None);
        assert_eq!(result.percentile_rtt(101.0), None);
    }

    #[test]
    fn test_percentile_empty_measurements() {
        let result = build_result(vec![]);
        assert_eq!(result.percentile_rtt(50.0), None);
    }

    #[test]
    fn test_percentile_all_dropped() {
        let result = build_result(vec![None, None, None]);
        assert_eq!(result.percentile_rtt(50.0), None);
    }

    #[test]
    fn tail_percentiles_match_distribution() {
        // RTTs 1..=100 ms. `percentile_rtt` uses nearest-rank over (len-1).
        let result = build_result((1..=100).map(|i| Some(i as f64)).collect());
        assert_eq!(result.p90_rtt(), Some(90.0));
        assert_eq!(result.p95_rtt(), Some(95.0));
        assert_eq!(result.p99_rtt(), Some(99.0));
        assert_eq!(result.p999_rtt(), Some(100.0));
    }

    #[test]
    fn spike_report_flags_injected_spikes() {
        // Flat 10 ms baseline with two large spikes. Threshold = max(30, 30) = 30.
        let mut rtts = vec![Some(10.0); 100];
        rtts[20] = Some(120.0);
        rtts[70] = Some(95.0);
        let sr = build_result(rtts).spike_report().expect("spike report");
        assert_eq!(sr.baseline_ms, 10.0);
        assert!((sr.threshold_ms - 30.0).abs() < 1e-9);
        assert_eq!(sr.spike_count, 2);
        assert!((sr.worst_ms - 120.0).abs() < 1e-9);
        // worst 120 ms ≥ 10× baseline → Frequent.
        assert_eq!(sr.verdict, SpikeVerdict::Frequent);
    }

    #[test]
    fn spike_report_occasional_for_mild_single_spike() {
        // One spike at 45 ms over a 10 ms baseline: 1% of probes, < 10× baseline.
        let mut rtts = vec![Some(10.0); 100];
        rtts[50] = Some(45.0);
        let sr = build_result(rtts).spike_report().expect("spike report");
        assert_eq!(sr.spike_count, 1);
        assert_eq!(sr.verdict, SpikeVerdict::Occasional);
    }

    #[test]
    fn spike_report_clean_when_flat() {
        let sr = build_result(vec![Some(12.0); 50])
            .spike_report()
            .expect("spike report");
        assert_eq!(sr.spike_count, 0);
        assert_eq!(sr.verdict, SpikeVerdict::Clean);
    }

    #[test]
    fn spike_report_none_without_samples() {
        assert!(build_result(vec![]).spike_report().is_none());
        assert!(build_result(vec![None, None]).spike_report().is_none());
    }
}
