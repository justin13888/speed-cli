use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::time::{Sleep, sleep};

/// Pacing mechanism to control packet transmission rate
#[derive(Debug)]
pub struct Pacer {
    /// Target sending rate in bytes per second
    sending_rate: f64,
    /// Last packet send time
    last_send_time: Option<Instant>,
    /// Accumulated debt from sending too fast
    debt: Duration,
    /// Next scheduled send time
    next_send_time: Option<Instant>,
}

impl Pacer {
    pub fn new(initial_rate: f64) -> Self {
        Self {
            sending_rate: initial_rate.max(1000.0), // Minimum 1KB/s
            last_send_time: None,
            debt: Duration::ZERO,
            next_send_time: None,
        }
    }

    /// Update the target sending rate
    pub fn update_rate(&mut self, rate: f64) {
        self.sending_rate = rate.max(1000.0);
    }

    /// Calculate when the next packet can be sent
    pub fn schedule_next_send(&mut self, packet_size: usize) -> Option<Duration> {
        let now = Instant::now();

        // Calculate the time this packet should take to send
        let send_duration = Duration::from_secs_f64(packet_size as f64 / self.sending_rate);

        match self.last_send_time {
            None => {
                // First packet, send immediately
                self.last_send_time = Some(now);
                self.next_send_time = Some(now + send_duration);
                None
            }
            Some(last_send) => {
                // Calculate when we should send based on pacing
                let ideal_send_time = last_send + send_duration;

                if now >= ideal_send_time {
                    // We can send now or we're already late
                    self.last_send_time = Some(now);
                    self.next_send_time = Some(now + send_duration);
                    None
                } else {
                    // We need to wait
                    let wait_time = ideal_send_time - now;
                    self.last_send_time = Some(ideal_send_time);
                    self.next_send_time = Some(ideal_send_time + send_duration);
                    Some(wait_time)
                }
            }
        }
    }

    /// Get the current sending rate
    pub fn get_rate(&self) -> f64 {
        self.sending_rate
    }
}

/// Future that resolves when it's time to send the next packet
pub struct PacedSend {
    sleep: Option<Pin<Box<Sleep>>>,
}

impl PacedSend {
    pub fn new(wait_duration: Option<Duration>) -> Self {
        Self {
            sleep: wait_duration.map(|d| Box::pin(sleep(d))),
        }
    }
}

impl Future for PacedSend {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.sleep.as_mut() {
            Some(sleep) => sleep.as_mut().poll(cx),
            None => Poll::Ready(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pacer_initialization() {
        let pacer = Pacer::new(10000.0); // 10KB/s
        assert_eq!(pacer.get_rate(), 10000.0);
    }

    #[test]
    fn test_pacer_rate_update() {
        let mut pacer = Pacer::new(10000.0);
        pacer.update_rate(20000.0);
        assert_eq!(pacer.get_rate(), 20000.0);
    }

    #[test]
    fn test_pacer_minimum_rate() {
        let pacer = Pacer::new(100.0); // Below minimum
        assert_eq!(pacer.get_rate(), 1000.0); // Should be clamped to minimum
    }

    #[tokio::test]
    async fn test_paced_send() {
        let paced_send = PacedSend::new(Some(Duration::from_millis(1)));
        let start = Instant::now();
        paced_send.await;
        let elapsed = start.elapsed();

        // Should have waited at least 1ms (with some tolerance for timing)
        assert!(elapsed >= Duration::from_millis(1));
    }
}
