use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Serialize};

use crate::report::{ConnectionError, ThroughputMeasurement};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputResult {
    /// Throughput measurements
    pub measurements: Vec<ThroughputMeasurement>,
    /// Total duration of the test
    pub total_duration: Duration,

    pub timestamp: DateTime<Utc>,
}

impl fmt::Display for ThroughputResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  {}: {}",
            "Data Transferred".bright_green().bold(),
            format_size(self.bytes_transferred(), BINARY).cyan()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Duration".bright_green().bold(),
            format!("{:.2}s", self.total_duration.as_secs_f64()).yellow()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Average Throughput".bright_green().bold(),
            format_size(
                (self.avg_throughput() * 8.0) as u64,
                DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"),
            )
            .magenta()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Connection Success Rate".bright_green().bold(),
            format!("{:.1}%", self.connection_success_rate() * 100.0).green()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Request Success Rate".bright_green().bold(),
            format!("{:.1}%", self.request_success_rate() * 100.0).green()
        )?;

        let (total_retries, successful_after_retry, failed_after_retry) = self.retry_statistics();
        if total_retries > 0 {
            writeln!(
                f,
                "  {}: {} (Success: {}, Failed: {})",
                "Total Retries".bright_green().bold(),
                total_retries.to_formatted_string(&Locale::en).yellow(),
                successful_after_retry
                    .to_formatted_string(&Locale::en)
                    .green(),
                failed_after_retry.to_formatted_string(&Locale::en).red()
            )?;

            writeln!(
                f,
                "  {}: {:.1}%",
                "Retry Success Rate".bright_green().bold(),
                self.retry_success_rate() * 100.0
            )?;
        }

        let error_distribution = self.error_distribution();
        if !error_distribution.is_empty() {
            writeln!(
                f,
                "  {}: {} total",
                "Errors".bright_green().bold(),
                self.total_errors().to_formatted_string(&Locale::en).red()
            )?;
            for (error_type, count) in error_distribution {
                writeln!(
                    f,
                    "    {}: {}",
                    error_type.bright_yellow(),
                    count.to_formatted_string(&Locale::en).red()
                )?;
            }
        }

        writeln!(
            f,
            "  {}: {}",
            "Measurements".bright_green().bold(),
            self.measurements
                .len()
                .to_formatted_string(&Locale::en)
                .white()
        )?;
        writeln!(
            f,
            "  {}: {}",
            "Timestamp".bright_green().bold(),
            self.timestamp
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .blue()
        )?;

        Ok(())
    }
}

impl ThroughputResult {
    /// Returns total number of bytes transferred
    pub fn bytes_transferred(&self) -> u64 {
        self.measurements
            .iter()
            .map(|m| match m {
                ThroughputMeasurement::Success { bytes, .. } => *bytes,
                ThroughputMeasurement::Failure { .. } => 0,
            })
            .sum()
    }

    /// Returns the average throughput in bytes per second
    pub fn avg_throughput(&self) -> f64 {
        if self.total_duration.is_zero() {
            return 0.0;
        }

        (self.bytes_transferred() as f64) / self.total_duration.as_secs_f64()
    }

    /// Returns the connection success rate as a percentage (0.0 to 1.0)
    pub fn connection_success_rate(&self) -> f64 {
        if self.measurements.is_empty() {
            return 0.0;
        }

        let successful_connections = self
            .measurements
            .iter()
            .filter(|m| matches!(m, ThroughputMeasurement::Success { .. }))
            .count();

        successful_connections as f64 / self.measurements.len() as f64
    }

    /// Returns the request success rate as a percentage (0.0 to 1.0)
    /// This is the same as connection success rate in this context
    pub fn request_success_rate(&self) -> f64 {
        self.connection_success_rate()
    }

    /// Returns retry statistics: (total_retries, successful_after_retry, failed_after_retry)
    pub fn retry_statistics(&self) -> (u32, u32, u32) {
        let mut total_retries = 0;
        let successful_after_retry = 0;
        let mut failed_after_retry = 0;

        for measurement in &self.measurements {
            match measurement {
                ThroughputMeasurement::Success { .. } => {
                    // For successful measurements, we assume no retries were needed
                    // This could be enhanced if success measurements tracked retry count
                }
                ThroughputMeasurement::Failure { retry_count, .. } => {
                    total_retries += retry_count;
                    failed_after_retry += 1;
                }
            }
        }

        (total_retries, successful_after_retry, failed_after_retry)
    }

    /// Returns the success rate after retries (0.0 to 1.0)
    pub fn retry_success_rate(&self) -> f64 {
        let (total_retries, successful_after_retry, failed_after_retry) = self.retry_statistics();

        if total_retries == 0 {
            return 1.0; // No retries needed means 100% success
        }

        successful_after_retry as f64 / (successful_after_retry + failed_after_retry) as f64
    }

    /// Returns error distribution by type
    pub fn error_distribution(&self) -> HashMap<String, u32> {
        let mut distribution = HashMap::new();

        for measurement in &self.measurements {
            if let ThroughputMeasurement::Failure { error, .. } = measurement {
                let error_type = match error {
                    ConnectionError::ConnectionFailed(_) => "Connection Failed",
                    ConnectionError::TransferFailed(_) => "Transfer Failed",
                    ConnectionError::Timeout(_) => "Timeout",
                    ConnectionError::Unknown(_) => "Unknown",
                };

                *distribution.entry(error_type.to_string()).or_insert(0) += 1;
            }
        }

        distribution
    }

    /// Returns the total number of errors
    pub fn total_errors(&self) -> u32 {
        self.measurements
            .iter()
            .filter(|m| matches!(m, ThroughputMeasurement::Failure { .. }))
            .count() as u32
    }
}
