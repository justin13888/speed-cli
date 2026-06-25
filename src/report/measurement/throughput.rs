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
                format!("{:.2}", self.duration_us as f64 / 1000.0).yellow(),
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
                format!("{:.2}", self.duration_us as f64 / 1000.0).yellow(),
                retry_count.to_string().yellow()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_millisecond_sample_is_not_truncated_to_zero_ms() {
        colored::control::set_override(false);
        // 500us used to render as "0 ms" via integer division; it must now
        // show fractional milliseconds.
        let s = Sample::success(0, 500, 1024, false);
        let rendered = format!("{s}");
        assert!(rendered.contains("0.50 ms"), "got: {rendered}");
    }

    #[test]
    fn failure_sample_round_trips_through_cbor() {
        // Regression: `ConnectionError` was internally tagged (`tag = "type"`),
        // which ciborium cannot serialize for newtype-string variants, so any
        // export containing a failed sample blew up with
        // "cannot serialize tagged newtype variant ... containing a string".
        let s = Sample::failure(0, 1_000, ConnectionError::Unknown("boom".into()), 2, false);

        let mut buf = Vec::new();
        ciborium::into_writer(&s, &mut buf).expect("CBOR encode must succeed");
        let back: Sample = ciborium::from_reader(buf.as_slice()).expect("CBOR decode");

        match back.outcome {
            Outcome::Failure { error, retry_count } => {
                assert_eq!(retry_count, 2);
                assert!(matches!(error, ConnectionError::Unknown(m) if m == "boom"));
            }
            Outcome::Success => panic!("expected a failure outcome"),
        }
    }
}
