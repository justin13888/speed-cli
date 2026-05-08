use std::fmt::{self, Display, Formatter};

use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};

use crate::report::LatencyMeasurement;

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
}
