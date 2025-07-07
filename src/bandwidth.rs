use std::time::{Duration, Instant};
use std::collections::VecDeque;

use crate::network::*;

pub struct BandwidthMonitor {
    measurements: VecDeque<BandwidthMeasurement>,
    window_size: Duration,
}

impl BandwidthMonitor {
    pub fn new(window_size: Duration) -> Self {
        Self {
            measurements: VecDeque::new(),
            window_size,
        }
    }
    
    /// Adds a new measurement to the monitor.
    pub fn add_measurement(&mut self, bytes: u64) {
        let measurement = BandwidthMeasurement::new(bytes);
        self.measurements.push_back(measurement);
        
        // Remove old measurements outside the window
        let cutoff_time = Instant::now() - self.window_size;
        while let Some(front) = self.measurements.front() {
            if front.instant < cutoff_time {
                self.measurements.pop_front();
            } else {
                break;
            }
        }
    }
    
    /// Calculates the current bandwidth in Mbps based on the first and last measurements.
    /// Returns 0.0 if there are not enough measurements.
    pub fn current_bandwidth_mbps(&self) -> f64 {
        if self.measurements.len() < 2 {
            return 0.0;
        }
        
        let first = &self.measurements[0];
        let last = &self.measurements[self.measurements.len() - 1];
        
        let duration = last.instant.duration_since(first.instant);
        let bytes_diff = last.bytes - first.bytes;
        
        if duration.as_secs_f64() > 0.0 {
            (bytes_diff as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0)
        } else {
            0.0
        }
    }
    
    /// Calculates the average bandwidth in Mbps over all measurements.
    /// Returns 0.0 if there are no measurements.
    pub fn average_bandwidth_mbps(&self) -> f64 {
        if self.measurements.is_empty() {
            return 0.0;
        }
        
        let total_bytes: u64 = self.measurements.iter().map(|m| m.bytes).sum();
        let avg_bytes = total_bytes as f64 / self.measurements.len() as f64;
        
        // Convert to Mbps (assuming measurements are per second)
        (avg_bytes * 8.0) / 1_000_000.0
    }
}

/// Calculates the jitter based on a slice of RTT samples.
pub fn calculate_jitter(rtt_samples: &[f64]) -> f64 {
    if rtt_samples.len() < 2 {
        return 0.0;
    }
    
    let mut variations = Vec::new();
    for i in 1..rtt_samples.len() {
        let variation = (rtt_samples[i] - rtt_samples[i - 1]).abs();
        variations.push(variation);
    }
    
    // Calculate average variation (jitter)
    variations.iter().sum::<f64>() / variations.len() as f64
}

#[derive(Debug)]
pub struct PacketLossTracker {
    /// Number of packets expected
    expected_packets: u32,

    /// Number of packets received
    received_packets: u32,
}

impl PacketLossTracker {
    pub fn new() -> Self {
        Self {
            expected_packets: 0,
            received_packets: 0,
        }
    }
    
    pub fn packet_sent(&mut self) {
        self.expected_packets += 1;
    }
    
    pub fn packet_received(&mut self) {
        self.received_packets += 1;
    }
    
    /// Returns the percentage of packet loss.
    /// If no packets were expected, returns 0.0.
    pub fn loss_percentage(&self) -> f64 {
        if self.expected_packets == 0 {
            0.0
        } else {
            let lost = self.expected_packets.saturating_sub(self.received_packets);
            (lost as f64 / self.expected_packets as f64) * 100.0
        }
    }
    
    /// Returns the number of packets lost.
    pub fn packets_lost(&self) -> u32 {
        self.expected_packets.saturating_sub(self.received_packets)
    }
}
