use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod config;
mod measurement;
mod result;

pub use config::*;
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
