use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::report::ThroughputMeasurement;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputResult {
    /// Throughput measurements
    pub measurements: Vec<ThroughputMeasurement>,
    /// Total duration of the test
    pub total_duration: Duration,

    pub timestamp: DateTime<Utc>,
}

impl fmt::Display for ThroughputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Test Result:\n  Bytes: {} MB\n  Duration: {:.2}s\n  Avg Throughput: {:.2} Mbps\n  Timestamp: {}",
            self.bytes_transferred() / 1_000_000,
            self.total_duration.as_secs_f64(),
            self.avg_throughput(),
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        )
    }
}

impl ThroughputResult {
    /// Returns total number of bytes transferred
    pub fn bytes_transferred(&self) -> u64 {
        self.measurements.iter().map(|m| m.bytes).sum()
    }

    /// Returns the average throughput in Mbps
    pub fn avg_throughput(&self) -> f64 {
        if self.total_duration.is_zero() {
            return 0.0;
        }

        (self.bytes_transferred() as f64 * 8.0) / (self.total_duration.as_secs_f64() * 1_000_000.0)
    }
}
