use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::report::{LatencyMeasurement, ThroughputMeasurement};
use crate::utils::format::{format_bytes, format_throughput};

/// Test type for progress bar styling
#[derive(Clone, Copy)]
pub enum ProgressBarType {
    Latency,
    Download,
    Upload,
}

/// Creates a progress bar with appropriate styling for the test type
pub fn create_progress_bar(test_type: ProgressBarType, duration: Duration) -> ProgressBar {
    let progress_bar = ProgressBar::new(duration.as_secs());

    let (color, test_name) = match test_type {
        ProgressBarType::Latency => ("yellow/blue", "latency"),
        ProgressBarType::Download => ("cyan/blue", "download"),
        ProgressBarType::Upload => ("green/blue", "upload"),
    };

    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "[{{elapsed_precise}}] {{bar:40.{color}}} {{pos}}s/{{len}}s {{msg}}"
            ))
            .unwrap()
            .progress_chars("##-"),
    );
    progress_bar.set_message(format!("{}...", test_name.to_title_case()));
    progress_bar
}

trait ToTitleCase {
    fn to_title_case(&self) -> String;
}

impl ToTitleCase for str {
    fn to_title_case(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().chain(chars).collect(),
        }
    }
}

/// Statistics collector for latency measurements
pub struct LatencyStatsCollector {
    pub measurements: Arc<std::sync::Mutex<Vec<LatencyMeasurement>>>,
    pub collector_handle: JoinHandle<()>,
    pub progress_handle: JoinHandle<()>,
}

impl LatencyStatsCollector {
    pub fn new(
        progress_bar: ProgressBar,
        start_time: Instant,
        duration: Duration,
    ) -> (Self, mpsc::UnboundedSender<LatencyMeasurement>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<LatencyMeasurement>();

        let measurements = Arc::new(std::sync::Mutex::new(Vec::<LatencyMeasurement>::new()));
        let measurements_clone = measurements.clone();

        // Collector task
        let collector_handle = tokio::spawn(async move {
            while let Some(measurement) = rx.recv().await {
                if let Ok(mut measurements) = measurements_clone.lock() {
                    measurements.push(measurement);
                }
            }
        });

        // Progress update task
        let progress_handle = {
            let pb = progress_bar.clone();
            let measurements_for_stats = measurements.clone();
            tokio::spawn(async move {
                while start_time.elapsed() < duration {
                    let elapsed = start_time.elapsed().as_secs();
                    pb.set_position(elapsed);

                    // Calculate and display running average latency (minimal overhead)
                    if let Ok(measurements) = measurements_for_stats.try_lock()
                        && !measurements.is_empty()
                    {
                        let valid_measurements: Vec<f64> =
                            measurements.iter().filter_map(|m| m.rtt_ms).collect();

                        if !valid_measurements.is_empty() {
                            let avg_latency = valid_measurements.iter().sum::<f64>()
                                / valid_measurements.len() as f64;
                            let connection_rate =
                                measurements.len() as f64 / start_time.elapsed().as_secs_f64();
                            pb.set_message(format!(
                                "Avg latency: {:.2}ms | Conn/s: {:.1} | Connections: {}",
                                avg_latency,
                                connection_rate,
                                measurements.len()
                            ));
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                pb.set_position(duration.as_secs());
            })
        };

        (
            Self {
                measurements,
                collector_handle,
                progress_handle,
            },
            tx,
        )
    }

    pub async fn finish(
        self,
        progress_bar: ProgressBar,
        message: String,
    ) -> Vec<LatencyMeasurement> {
        let _ = tokio::join!(self.collector_handle, self.progress_handle);
        progress_bar.finish_with_message(message);

        // Extract measurements with minimal lock time

        self.measurements.lock().unwrap().clone()
    }
}

/// Statistics collector for throughput measurements
pub struct ThroughputStatsCollector {
    pub measurements: Arc<std::sync::Mutex<Vec<ThroughputMeasurement>>>,
    pub collector_handle: JoinHandle<()>,
    pub progress_handle: JoinHandle<()>,
}

impl ThroughputStatsCollector {
    pub fn new(
        progress_bar: ProgressBar,
        start_time: Instant,
        duration: Duration,
    ) -> (Self, mpsc::UnboundedSender<ThroughputMeasurement>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<ThroughputMeasurement>();

        let measurements = Arc::new(std::sync::Mutex::new(Vec::<ThroughputMeasurement>::new()));
        let measurements_clone = measurements.clone();

        // Collector task
        let collector_handle = tokio::spawn(async move {
            while let Some(measurement) = rx.recv().await {
                if let Ok(mut measurements) = measurements_clone.lock() {
                    measurements.push(measurement);
                }
            }
        });

        // Progress update task
        let progress_handle = {
            let pb = progress_bar.clone();
            let measurements_for_stats = measurements.clone();
            tokio::spawn(async move {
                while start_time.elapsed() < duration {
                    let elapsed = start_time.elapsed().as_secs();
                    pb.set_position(elapsed);

                    // Calculate and display running average throughput (minimal overhead)
                    if let Ok(measurements) = measurements_for_stats.try_lock()
                        && !measurements.is_empty()
                    {
                        let total_bytes: u64 = measurements
                            .iter()
                            .map(|m| match m {
                                ThroughputMeasurement::Success { bytes, .. } => *bytes,
                                ThroughputMeasurement::Failure { .. } => 0,
                            })
                            .sum();
                        let elapsed_secs = start_time.elapsed().as_secs_f64();
                        let throughput_mbps =
                            (total_bytes as f64 * 8.0) / (elapsed_secs * 1_000_000.0);
                        let throughput_bytes_per_sec = total_bytes as f64 / elapsed_secs;
                        let requests_per_sec = measurements.len() as f64 / elapsed_secs;

                        pb.set_message(format!(
                            "Avg: {} | {} | Req/s: {:.1} | Chunks: {}",
                            format_throughput(throughput_mbps),
                            format_bytes(throughput_bytes_per_sec as usize),
                            requests_per_sec,
                            measurements.len()
                        ));
                    }

                    // Update every 100ms
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                pb.set_position(duration.as_secs());
            })
        };

        (
            Self {
                measurements,
                collector_handle,
                progress_handle,
            },
            tx,
        )
    }

    pub async fn finish(
        self,
        progress_bar: ProgressBar,
        message: String,
    ) -> Vec<ThroughputMeasurement> {
        let _ = tokio::join!(self.collector_handle, self.progress_handle);
        progress_bar.finish_with_message(message);

        // Extract measurements with minimal lock time

        self.measurements.lock().unwrap().clone()
    }
}
