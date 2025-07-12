use std::time::Duration;

use chrono::{DateTime, Utc};
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

    /// Returns total elapsed time for all measurements
    pub fn total_elapsed(&self) -> Duration {
        // Take the maximum elapsed time from all measurements
        self.measurements
            .iter()
            .map(|m| m.elapsed_time)
            .max()
            .unwrap_or(Duration::ZERO)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMeasurement {
    /// RTT in milliseconds. If dropped, it is None.
    pub rtt_ms: Option<f64>,
    pub elapsed_time: Duration,
}
