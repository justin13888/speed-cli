use std::fmt::{self, Display, Formatter};

use colored::*;
use serde::{Deserialize, Serialize};

/// A single latency observation.
///
/// `t_start_us` is the offset in microseconds from `TestReport.start_time`
/// to when the probe was sent. `rtt_us` is `None` for dropped probes
/// (timeout, send error, malformed echo). Time-ordered placement of
/// samples on the test's duration axis is the same convention used by
/// `Sample` (throughput).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMeasurement {
    pub t_start_us: u64,
    pub rtt_us: Option<u64>,
}

impl LatencyMeasurement {
    pub fn success(t_start_us: u64, rtt_us: u64) -> Self {
        Self {
            t_start_us,
            rtt_us: Some(rtt_us),
        }
    }

    pub fn dropped(t_start_us: u64) -> Self {
        Self {
            t_start_us,
            rtt_us: None,
        }
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.rtt_us.map(|us| us as f64 / 1000.0)
    }
}

impl Display for LatencyMeasurement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.rtt_ms() {
            Some(rtt) => write!(f, "{rtt:.2} ms"),
            None => write!(f, "{}", "dropped".red()),
        }
    }
}
