use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size, format_size_i};
use serde::{Deserialize, Serialize};

use crate::report::ConnectionError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ThroughputMeasurement {
    #[serde(rename = "success")]
    Success {
        /// Number of bytes transferred
        bytes: u64,
        /// Duration of transfer
        duration: Duration,
    },
    #[serde(rename = "failure")]
    Failure {
        /// Error that occurred
        error: ConnectionError,
        /// Duration before failure
        duration: Duration,
        /// Number of retry attempts before failure
        retry_count: u32,
    },
}

impl ThroughputMeasurement {
    pub fn new(bytes: u64, duration: Duration) -> Self {
        Self::Success { bytes, duration }
    }

    pub fn new_error(error: ConnectionError, duration: Duration, retry_count: u32) -> Self {
        Self::Failure {
            error,
            duration,
            retry_count,
        }
    }

    /// Returns throughput in bits per second if successful, otherwise returns 0.0
    pub fn throughput_bps(&self) -> f64 {
        match self {
            Self::Success { bytes, duration } => {
                if duration.as_secs_f64() > 0.0 {
                    (*bytes as f64 * 8.0) / duration.as_secs_f64()
                } else {
                    0.0
                }
            }
            Self::Failure { .. } => 0.0,
        }
    }
}

impl Display for ThroughputMeasurement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success { bytes, duration } => {
                write!(
                    f,
                    "{} in {} ms ({})",
                    format_size(*bytes, BINARY).cyan(),
                    duration.as_millis().to_string().yellow(),
                    format_size_i(
                        self.throughput_bps(),
                        DECIMAL.base_unit(BaseUnit::Bit).suffix("/s")
                    )
                    .magenta()
                )
            }
            Self::Failure {
                error,
                duration,
                retry_count,
            } => {
                write!(
                    f,
                    "{}: {} (after {} ms, {} retries)",
                    "Error".red(),
                    error,
                    duration.as_millis().to_string().yellow(),
                    retry_count.to_string().yellow()
                )
            }
        }
    }
}
