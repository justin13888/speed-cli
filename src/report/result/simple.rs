use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use humansize::{BINARY, BaseUnit, DECIMAL, format_size};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Serialize};

use crate::report::ThroughputMeasurement;
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
                self.avg_throughput() as u64,
                DECIMAL.base_unit(BaseUnit::Bit).suffix("/s"),
            )
            .magenta()
        )?;
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
        self.measurements.iter().map(|m| m.bytes).sum()
    }

    /// Returns the average throughput in bytes per second
    pub fn avg_throughput(&self) -> f64 {
        if self.total_duration.is_zero() {
            return 0.0;
        }

        (self.bytes_transferred() as f64) / self.total_duration.as_secs_f64()
    }
}
