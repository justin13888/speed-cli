use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputMeasurement {
    /// Number of bytes transferred
    pub bytes: u64,
    /// Duration of transfer
    pub duration: Duration,
}

impl ThroughputMeasurement {
    pub fn new(bytes: u64, duration: Duration) -> Self {
        Self { bytes, duration }
    }
}

impl Display for ThroughputMeasurement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let bytes_kb = self.bytes as f64 / 1024.0;
        let duration_ms = self.duration.as_millis();
        let throughput_kbps = if self.duration.as_secs_f64() > 0.0 {
            (self.bytes as f64 * 8.0) / (self.duration.as_secs_f64() * 1024.0)
        } else {
            0.0
        };

        write!(
            f,
            "{} KB in {} ms ({throughput_kbps:.2} Kbps)",
            format!("{bytes_kb:.2}").cyan(),
            duration_ms.to_string().yellow()
        )
    }
}
