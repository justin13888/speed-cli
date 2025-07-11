use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleTestResult {
    pub bytes_transferred: u64,
    pub duration: Duration,
    pub bandwidth_mbps: f64,
    pub jitter_ms: Option<f64>,
    pub packet_loss: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

impl SimpleTestResult {
    pub fn new(bytes: u64, duration: Duration) -> Self {
        let bandwidth_mbps = (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);

        Self {
            bytes_transferred: bytes,
            duration,
            bandwidth_mbps,
            jitter_ms: None,
            packet_loss: None,
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn with_jitter(mut self, jitter_ms: f64) -> Self {
        self.jitter_ms = Some(jitter_ms);
        self
    }

    pub fn with_packet_loss(mut self, loss_percent: f64) -> Self {
        self.packet_loss = Some(loss_percent);
        self
    }
}
