use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size, format_size_i};
use serde::{Deserialize, Serialize};

use crate::report::ConnectionError;

/// A single throughput observation.
///
/// `t_start_us` and `duration_us` describe a half-open interval
/// `[t_start_us, t_start_us + duration_us)` on the test's monotonic
/// time axis, where `0` is `TestReport.start_time`. This is what
/// makes a stability graph possible: every sample knows where on
/// the duration axis it lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub t_start_us: u64,
    pub duration_us: u64,
    pub bytes: u64,
    pub outcome: Outcome,
    pub is_warmup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Outcome {
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "failure")]
    Failure {
        error: ConnectionError,
        retry_count: u32,
    },
}

impl Sample {
    pub fn success(t_start_us: u64, duration_us: u64, bytes: u64, is_warmup: bool) -> Self {
        Self {
            t_start_us,
            duration_us,
            bytes,
            outcome: Outcome::Success,
            is_warmup,
        }
    }

    pub fn failure(
        t_start_us: u64,
        duration_us: u64,
        error: ConnectionError,
        retry_count: u32,
        is_warmup: bool,
    ) -> Self {
        Self {
            t_start_us,
            duration_us,
            bytes: 0,
            outcome: Outcome::Failure { error, retry_count },
            is_warmup,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self.outcome, Outcome::Success)
    }

    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.duration_us)
    }

    /// Throughput of this single sample in bits per second. Returns 0.0
    /// for failures or zero-duration samples.
    pub fn throughput_bps(&self) -> f64 {
        if self.is_success() && self.duration_us > 0 {
            (self.bytes as f64 * 8.0) / (self.duration_us as f64 / 1_000_000.0)
        } else {
            0.0
        }
    }
}

impl Display for Sample {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.outcome {
            Outcome::Success => write!(
                f,
                "{} in {} ms ({})",
                format_size(self.bytes, BINARY).cyan(),
                (self.duration_us / 1000).to_string().yellow(),
                format_size_i(
                    self.throughput_bps(),
                    DECIMAL.base_unit(BaseUnit::Bit).suffix("/s")
                )
                .magenta()
            ),
            Outcome::Failure { error, retry_count } => write!(
                f,
                "{}: {} (after {} ms, {} retries)",
                "Error".red(),
                error,
                (self.duration_us / 1000).to_string().yellow(),
                retry_count.to_string().yellow()
            ),
        }
    }
}
