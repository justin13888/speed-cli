use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size, format_size_i};
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

    /// Returns throughput in bits per second
    pub fn throughput_bps(&self) -> f64 {
        if self.duration.as_secs_f64() > 0.0 {
            (self.bytes as f64 * 8.0) / self.duration.as_secs_f64()
        } else {
            0.0
        }
    }
}

impl Display for ThroughputMeasurement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} in {} ms ({})",
            format_size(self.bytes, BINARY).cyan(),
            self.duration.as_millis().to_string().yellow(),
            format_size_i(
                self.throughput_bps(),
                DECIMAL.base_unit(BaseUnit::Bit).suffix("/s")
            )
            .magenta()
        )
    }
}
