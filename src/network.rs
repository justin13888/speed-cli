use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use humansize::{format_size, BINARY};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub bytes_transferred: u64,
    pub duration: Duration,
    pub bandwidth_mbps: f64,
    pub jitter_ms: Option<f64>,
    pub packet_loss: Option<f64>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl TestResult {
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

#[derive(Debug, Clone)]
pub struct BandwidthMeasurement {
    pub instant: Instant,
    pub bytes: u64,
}

impl BandwidthMeasurement {
    pub fn new(bytes: u64) -> Self {
        Self {
            instant: Instant::now(),
            bytes,
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

pub fn format_bandwidth(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.2} Gbps", mbps / 1000.0)
    } else {
        format!("{mbps:.2} Mbps")
    }
}
