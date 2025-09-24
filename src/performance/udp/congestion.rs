use std::collections::VecDeque;
use std::time::{Duration, Instant};

// TODO: Check the logic for correctness here...

/// Trait for congestion control algorithms
pub trait CongestionControl {
    /// Called when a packet is sent
    fn on_packet_sent(&mut self, bytes_sent: usize, now: Instant);

    /// Called when an ACK is received
    fn on_ack_received(&mut self, bytes_acked: usize, rtt: Duration, now: Instant);

    /// Called when packet loss is detected
    fn on_packet_lost(&mut self, bytes_lost: usize, now: Instant);

    /// Get current sending rate in bytes per second
    fn get_sending_rate(&self) -> f64;

    /// Get current congestion window in bytes
    fn get_cwnd(&self) -> usize;

    /// Check if we can send more data
    fn can_send(&self, bytes_in_flight: usize) -> bool;
}

/// BBR congestion control algorithm implementation
/// Based on "BBR: Congestion-Based Congestion Control" by Cardwell et al.
#[derive(Debug)]
pub struct BbrCongestionControl {
    // Core BBR state
    state: BbrState,

    // Bandwidth estimation
    max_bw: BandwidthFilter,

    // RTT estimation
    min_rtt: Duration,
    min_rtt_stamp: Instant,

    // Pacing and cwnd
    pacing_rate: f64, // bytes per second
    cwnd: usize,

    // Cycle tracking for probing
    cycle_index: usize,
    cycle_start: Instant,

    // Packet counting for gain cycling
    packets_sent: u64,
    packets_acked: u64,

    // Configuration
    config: BbrConfig,

    // State timing
    state_start: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BbrState {
    Startup,
    Drain,
    ProbeBw,
    ProbeRtt,
}

#[derive(Debug)]
struct BandwidthFilter {
    samples: VecDeque<BandwidthSample>,
    max_bw: f64,
}

#[derive(Debug, Clone)]
struct BandwidthSample {
    bw: f64,
    time: Instant,
}

#[derive(Debug, Clone)]
struct BbrConfig {
    // Startup phase
    startup_cwnd_gain: f64,
    startup_pacing_gain: f64,

    // ProbeBW phase cycling gains
    probe_bw_gains: Vec<f64>,
    probe_bw_cycle_len: Duration,

    // ProbeRTT
    probe_rtt_duration: Duration,
    probe_rtt_cwnd_gain: f64,

    // General
    min_cwnd: usize,
    max_cwnd: usize,
    initial_cwnd: usize,

    // RTT filter window
    min_rtt_filter_len: Duration,

    // Bandwidth filter
    bw_filter_len: Duration,
}

impl Default for BbrConfig {
    fn default() -> Self {
        Self {
            startup_cwnd_gain: 2.0,
            startup_pacing_gain: 2.77, // ln(2) * 4

            probe_bw_gains: vec![1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            probe_bw_cycle_len: Duration::from_secs(8),

            probe_rtt_duration: Duration::from_millis(200),
            probe_rtt_cwnd_gain: 0.5,

            min_cwnd: 4 * 1400,      // 4 packets of 1400 bytes
            max_cwnd: 1024 * 1024,   // 1MB
            initial_cwnd: 10 * 1400, // 10 packets

            min_rtt_filter_len: Duration::from_secs(10),
            bw_filter_len: Duration::from_secs(2),
        }
    }
}

impl BandwidthFilter {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            max_bw: 0.0,
        }
    }

    fn update(&mut self, bw: f64, now: Instant, filter_len: Duration) {
        // Add new sample
        self.samples.push_back(BandwidthSample { bw, time: now });

        // Remove old samples
        let cutoff = now - filter_len;
        while let Some(sample) = self.samples.front() {
            if sample.time < cutoff {
                self.samples.pop_front();
            } else {
                break;
            }
        }

        // Update max bandwidth
        self.max_bw = self.samples.iter().map(|s| s.bw).fold(0.0, f64::max);
    }

    fn get_max_bw(&self) -> f64 {
        self.max_bw
    }
}

impl BbrCongestionControl {
    pub fn new() -> Self {
        let now = Instant::now();
        let config = BbrConfig::default();

        Self {
            state: BbrState::Startup,
            max_bw: BandwidthFilter::new(),
            min_rtt: Duration::from_millis(1000), // Conservative initial estimate
            min_rtt_stamp: now,
            pacing_rate: 0.0,
            cwnd: config.initial_cwnd,
            cycle_index: 0,
            cycle_start: now,
            packets_sent: 0,
            packets_acked: 0,
            config,
            state_start: now,
        }
    }

    fn update_model(&mut self, bytes_acked: usize, rtt: Duration, now: Instant) {
        // Update RTT
        if rtt < self.min_rtt || now - self.min_rtt_stamp > self.config.min_rtt_filter_len {
            self.min_rtt = rtt;
            self.min_rtt_stamp = now;
        }

        // Update bandwidth (simple rate calculation)
        let delivery_rate = bytes_acked as f64 / rtt.as_secs_f64();
        self.max_bw
            .update(delivery_rate, now, self.config.bw_filter_len);
    }

    fn update_control_parameters(&mut self, _now: Instant) {
        match self.state {
            BbrState::Startup => {
                self.pacing_rate = self.config.startup_pacing_gain * self.max_bw.get_max_bw();
                self.cwnd = (self.config.startup_cwnd_gain
                    * (self.max_bw.get_max_bw() * self.min_rtt.as_secs_f64()))
                    as usize;
                self.cwnd = self
                    .cwnd
                    .max(self.config.min_cwnd)
                    .min(self.config.max_cwnd);
            }
            BbrState::Drain => {
                self.pacing_rate = self.max_bw.get_max_bw() / self.config.startup_pacing_gain;
                self.cwnd = (self.max_bw.get_max_bw() * self.min_rtt.as_secs_f64()) as usize;
                self.cwnd = self
                    .cwnd
                    .max(self.config.min_cwnd)
                    .min(self.config.max_cwnd);
            }
            BbrState::ProbeBw => {
                let gain = self.get_probe_bw_gain();
                self.pacing_rate = gain * self.max_bw.get_max_bw();
                self.cwnd = ((gain * self.max_bw.get_max_bw() * self.min_rtt.as_secs_f64())
                    as usize)
                    .max(self.config.min_cwnd)
                    .min(self.config.max_cwnd);
            }
            BbrState::ProbeRtt => {
                self.pacing_rate = self.max_bw.get_max_bw();
                self.cwnd = (self.config.probe_rtt_cwnd_gain
                    * self.max_bw.get_max_bw()
                    * self.min_rtt.as_secs_f64()) as usize;
                self.cwnd = self
                    .cwnd
                    .max(self.config.min_cwnd)
                    .min(self.config.max_cwnd);
            }
        }
    }

    fn get_probe_bw_gain(&self) -> f64 {
        self.config.probe_bw_gains[self.cycle_index % self.config.probe_bw_gains.len()]
    }

    fn advance_cycle_phase(&mut self, now: Instant) {
        if self.state != BbrState::ProbeBw {
            return;
        }

        if now - self.cycle_start
            >= self.config.probe_bw_cycle_len / self.config.probe_bw_gains.len() as u32
        {
            self.cycle_index = (self.cycle_index + 1) % self.config.probe_bw_gains.len();
            self.cycle_start = now;
        }
    }

    fn check_startup_done(&mut self) -> bool {
        // Exit startup if we haven't increased bandwidth significantly in recent RTTs
        // This is a simplified check - real BBR has more sophisticated logic
        self.packets_acked > 3 * (self.cwnd / 1400) as u64
    }

    fn check_drain_done(&mut self, bytes_in_flight: usize) -> bool {
        // Exit drain when bytes in flight drops below BDP
        let bdp = (self.max_bw.get_max_bw() * self.min_rtt.as_secs_f64()) as usize;
        bytes_in_flight <= bdp
    }

    fn update_state(&mut self, bytes_in_flight: usize, now: Instant) {
        match self.state {
            BbrState::Startup => {
                if self.check_startup_done() {
                    self.state = BbrState::Drain;
                    self.state_start = now;
                }
            }
            BbrState::Drain => {
                if self.check_drain_done(bytes_in_flight) {
                    self.state = BbrState::ProbeBw;
                    self.state_start = now;
                    self.cycle_start = now;
                    self.cycle_index = 0;
                }
            }
            BbrState::ProbeBw => {
                // Check if we should enter ProbeRTT
                if now - self.min_rtt_stamp > self.config.min_rtt_filter_len {
                    self.state = BbrState::ProbeRtt;
                    self.state_start = now;
                }
                self.advance_cycle_phase(now);
            }
            BbrState::ProbeRtt => {
                if now - self.state_start >= self.config.probe_rtt_duration {
                    self.state = BbrState::ProbeBw;
                    self.state_start = now;
                    self.cycle_start = now;
                    self.cycle_index = 0;
                }
            }
        }
    }
}

impl CongestionControl for BbrCongestionControl {
    fn on_packet_sent(&mut self, _bytes_sent: usize, now: Instant) {
        self.packets_sent += 1;
        self.update_state(0, now); // We don't track bytes_in_flight here
        self.update_control_parameters(now);
    }

    fn on_ack_received(&mut self, bytes_acked: usize, rtt: Duration, now: Instant) {
        self.packets_acked += 1;
        self.update_model(bytes_acked, rtt, now);
        self.update_state(0, now); // We don't track bytes_in_flight here
        self.update_control_parameters(now);
    }

    fn on_packet_lost(&mut self, _bytes_lost: usize, now: Instant) {
        // BBR is less reactive to individual losses compared to loss-based algorithms
        // We mainly rely on bandwidth and RTT measurements
        self.update_state(0, now);
        self.update_control_parameters(now);
    }

    fn get_sending_rate(&self) -> f64 {
        self.pacing_rate.max(1000.0) // Minimum 1KB/s
    }

    fn get_cwnd(&self) -> usize {
        self.cwnd
    }

    fn can_send(&self, bytes_in_flight: usize) -> bool {
        bytes_in_flight < self.cwnd
    }
}

impl Default for BbrCongestionControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbr_initialization() {
        let bbr = BbrCongestionControl::new();
        assert!(bbr.get_cwnd() > 0);
        assert!(bbr.get_sending_rate() >= 1000.0);
    }

    #[test]
    fn test_bandwidth_filter() {
        let mut filter = BandwidthFilter::new();
        let now = Instant::now();

        filter.update(1000.0, now, Duration::from_secs(1));
        filter.update(
            2000.0,
            now + Duration::from_millis(100),
            Duration::from_secs(1),
        );
        filter.update(
            1500.0,
            now + Duration::from_millis(200),
            Duration::from_secs(1),
        );

        assert_eq!(filter.get_max_bw(), 2000.0);
    }
}
