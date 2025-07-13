use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    /// List of RTT measurements in milliseconds
    pub measurements: Vec<LatencyMeasurement>,
    pub timestamp: DateTime<Utc>,
}

impl LatencyResult {
    /// Returns list of all RTT measurements that are not None
    pub fn rtts(&self) -> Vec<f64> {
        self.measurements.iter().filter_map(|m| m.rtt_ms).collect()
    }

    /// Total number of measurements
    pub fn count(&self) -> usize {
        self.measurements.len()
    }

    /// Number of successful measurements (where RTT is not None)
    pub fn successful_count(&self) -> usize {
        self.measurements
            .iter()
            .filter(|m| m.rtt_ms.is_some())
            .count()
    }

    /// Returns number of dropped measurements (where RTT is None)
    pub fn dropped_count(&self) -> usize {
        self.measurements
            .iter()
            .filter(|m| m.rtt_ms.is_none())
            .count()
    }

    /// Returns average RTT. If no measurements, returns 0.0
    pub fn avg_rtt(&self) -> Option<f64> {
        let mut sum = 0.0;
        let mut count = 0;

        for measurement in self.measurements.iter() {
            if let Some(rtt) = measurement.rtt_ms {
                sum += rtt;
                count += 1;
            }
        }

        if count > 0 {
            Some(sum / count as f64)
        } else {
            None
        }
    }

    /// Returns minimum RTT if available, otherwise None.
    pub fn min_rtt(&self) -> Option<f64> {
        self.rtts().into_iter().fold(None, |acc, rtt| {
            Some(acc.map_or(rtt, |current_min| rtt.min(current_min)))
        })
    }

    /// Returns n-th percentile RTT.
    /// If n is out of bounds, returns None.
    pub fn percentile_rtt(&self, n: f64) -> Option<f64> {
        if !(0.0..=100.0).contains(&n) {
            return None;
        }
        let mut rtts = self.rtts();
        rtts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let index = ((n / 100.0) * rtts.len() as f64).round() as usize;
        if index < rtts.len() {
            Some(rtts[index])
        } else {
            None
        }
    }

    /// Returns maximum RTT if available, otherwise None.
    pub fn max_rtt(&self) -> Option<f64> {
        self.rtts().into_iter().fold(None, |acc, rtt| {
            Some(acc.map_or(rtt, |current_max| rtt.max(current_max)))
        })
    }

    /// Returns jitter (standard deviation of RTT)
    /// If no measurements, returns None
    pub fn jitter(&self) -> Option<f64> {
        let rtts = self.rtts();
        if rtts.is_empty() {
            return None;
        }
        let mean = self.avg_rtt()?;
        let variance =
            rtts.iter().map(|&rtt| (rtt - mean).powi(2)).sum::<f64>() / rtts.len() as f64;
        Some(variance.sqrt())
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

        if let Some(jitter) = self.jitter() {
            writeln!(
                f,
                "    {}: {}",
                "Jitter".bright_blue().bold(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMeasurement {
    /// RTT in milliseconds. If dropped, it is None.
    pub rtt_ms: Option<f64>,
    pub elapsed_time: Duration,
}

impl Display for LatencyMeasurement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.rtt_ms {
            Some(rtt) => write!(f, "{rtt:.2} ms"),
            None => write!(f, "{}", "dropped".red()),
        }
    }
}

// TODO: Add unit test for nth percentile calculation
