//! Generic background stats collector.
//!
//! Drains samples from an unbounded channel into a shared buffer on one task
//! while a second task refreshes a live progress bar. One generic
//! implementation replaces the previously-duplicated throughput and latency
//! collectors; the per-type live-stat formatting is supplied by [`LiveStat`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use indicatif::ProgressBar;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::report::{LatencyMeasurement, Sample};
use crate::utils::format::{format_bytes, format_throughput};

/// A sample type that can summarize a live one-line progress message.
pub trait LiveStat: Send + 'static {
    /// Render the progress-bar message from all samples collected so far,
    /// given the wall-clock `elapsed` since the test started. Return an empty
    /// string to leave the current message unchanged.
    fn render(samples: &[Self], elapsed: Duration) -> String
    where
        Self: Sized;
}

impl LiveStat for Sample {
    fn render(samples: &[Self], elapsed: Duration) -> String {
        let total_bytes: u64 = samples
            .iter()
            .filter(|s| !s.is_warmup && s.is_success())
            .map(|s| s.bytes)
            .sum();
        let secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
        let throughput_mbps = (total_bytes as f64 * 8.0) / (secs * 1_000_000.0);
        let bytes_per_sec = total_bytes as f64 / secs;
        let requests_per_sec = samples.len() as f64 / secs;
        format!(
            "Avg: {} | {} | Req/s: {:.1} | Samples: {}",
            format_throughput(throughput_mbps),
            format_bytes(bytes_per_sec as usize),
            requests_per_sec,
            samples.len()
        )
    }
}

impl LiveStat for LatencyMeasurement {
    fn render(samples: &[Self], elapsed: Duration) -> String {
        let valid: Vec<f64> = samples.iter().filter_map(|m| m.rtt_ms()).collect();
        if valid.is_empty() {
            return String::new();
        }
        let avg_latency = valid.iter().sum::<f64>() / valid.len() as f64;
        let secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
        let connection_rate = samples.len() as f64 / secs;
        format!(
            "Avg latency: {avg_latency:.2}ms | Conn/s: {connection_rate:.1} | Connections: {}",
            samples.len()
        )
    }
}

/// Background collector: drains `T`s from a channel into a shared buffer and
/// drives a live progress bar until the test window elapses.
pub struct StatsCollector<T: LiveStat> {
    samples: Arc<Mutex<Vec<T>>>,
    collector_handle: JoinHandle<()>,
    progress_handle: JoinHandle<()>,
}

/// Collector for bulk-transfer throughput samples.
pub type ThroughputStatsCollector = StatsCollector<Sample>;
/// Collector for latency probe measurements.
pub type LatencyStatsCollector = StatsCollector<LatencyMeasurement>;

impl<T: LiveStat> StatsCollector<T> {
    /// Spawn the collector + progress tasks. Returns the collector handle and
    /// the sender that test tasks push samples into; drop all senders to let
    /// the collector drain and exit.
    pub fn new(
        progress_bar: ProgressBar,
        start_time: Instant,
        duration: Duration,
    ) -> (Self, mpsc::UnboundedSender<T>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<T>();
        let samples = Arc::new(Mutex::new(Vec::<T>::new()));

        let collector_samples = samples.clone();
        let collector_handle = tokio::spawn(async move {
            while let Some(sample) = rx.recv().await {
                collector_samples.lock().push(sample);
            }
        });

        let progress_handle = {
            let pb = progress_bar.clone();
            let stat_samples = samples.clone();
            tokio::spawn(async move {
                while start_time.elapsed() < duration {
                    pb.set_position(start_time.elapsed().as_secs());
                    let message = {
                        let guard = stat_samples.lock();
                        if guard.is_empty() {
                            None
                        } else {
                            Some(T::render(&guard, start_time.elapsed()))
                        }
                    };
                    if let Some(message) = message
                        && !message.is_empty()
                    {
                        pb.set_message(message);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                pb.set_position(duration.as_secs());
            })
        };

        (
            Self {
                samples,
                collector_handle,
                progress_handle,
            },
            tx,
        )
    }

    /// Await both background tasks, finalize the progress bar, and return the
    /// collected samples. Uses `parking_lot` so a panicked producer cannot
    /// poison the buffer and turn this into a panic.
    pub async fn finish(self, progress_bar: ProgressBar, message: String) -> Vec<T> {
        let _ = tokio::join!(self.collector_handle, self.progress_handle);
        progress_bar.finish_with_message(message);
        // After the tasks join, their `Arc` clones are dropped, so the unwrap
        // succeeds and we move the buffer out without cloning.
        Arc::try_unwrap(self.samples)
            .map(Mutex::into_inner)
            .unwrap_or_else(|arc| std::mem::take(&mut arc.lock()))
    }
}
