use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    pub avg_rtt: f64,
    pub min_rtt: f64,
    pub max_rtt: f64,
    pub jitter: f64,
    pub measurements: Vec<LatencyMeasurement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMeasurement {
    pub rtt_ms: f64,
    pub elapsed_time: Duration,
}
