use std::time::Duration;

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
