use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

mod config;
mod errors;
mod measurement;
mod result;

pub use config::*;
pub use errors::*;
pub use measurement::*;
pub use result::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    /// Start time
    pub start_time: DateTime<Utc>,
    /// Test configuration
    pub config: TestConfig,
    /// Test result
    pub result: TestResult,
    /// Report timestamp in seconds
    pub timestamp: DateTime<Utc>,
    /// Version of speed-cli that generated this report
    pub version: String,
}

impl TestReport {
    pub fn new(
        start_time: DateTime<Utc>,
        config: TestConfig,
        result: TestResult,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            start_time,
            config,
            result,
            timestamp,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl<T, C, R> From<(T, C, R, T)> for TestReport
where
    T: Into<DateTime<Utc>>,
    C: Into<TestConfig>,
    R: Into<TestResult>,
{
    fn from((start_time, config, result, timestamp): (T, C, R, T)) -> Self {
        Self {
            start_time: start_time.into(),
            config: config.into(),
            result: result.into(),
            timestamp: timestamp.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl<T, C, R> From<(T, C, R)> for TestReport
where
    T: Into<DateTime<Utc>>,
    C: Into<TestConfig>,
    R: Into<TestResult>,
{
    fn from((start_time, config, result): (T, C, R)) -> Self {
        Self::new(start_time.into(), config.into(), result.into(), Utc::now())
    }
}

impl Display for TestReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}",
            "═══ Speed CLI Test Report ═══".bright_cyan().bold()
        )?;
        writeln!(
            f,
            "{}: {}",
            "Version".bright_white().bold(),
            self.version.green()
        )?;
        writeln!(
            f,
            "{}: {}",
            "Start Time".bright_white().bold(),
            self.start_time
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .yellow()
        )?;
        writeln!(
            f,
            "{}: {}",
            "Report Time".bright_white().bold(),
            self.timestamp
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .yellow()
        )?;
        writeln!(f)?;

        writeln!(f, "{}", "Configuration:".bright_white().bold().underline())?;
        write!(f, "{}", self.config)?;
        writeln!(f)?;

        writeln!(f, "{}", "Results:".bright_white().bold().underline())?;
        write!(f, "{}", self.result)?;

        Ok(())
    }
}
