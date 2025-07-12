
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use config::*;
pub use result::*;

mod config;
mod result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    /// Test configuration
    pub config: TestConfig,
    /// Test result
    pub result: TestResult,
    /// Report timestamp in seconds
    pub timestamp: DateTime<Utc>,
    /// Version of speed-cli that generated this report
    pub version: String,
}

impl<C, R, T> From<(C, R, T)> for TestReport
where
    C: Into<TestConfig>,
    R: Into<TestResult>,
    T: Into<DateTime<Utc>>,
{
    fn from((config, result, timestamp): (C, R, T)) -> Self {
        Self {
            config: config.into(),
            result: result.into(),
            timestamp: timestamp.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl<C, R> From<(C, R)> for TestReport
where
    C: Into<TestConfig>,
    R: Into<TestResult>,
{
    fn from((config, result): (C, R)) -> Self {
        (config, result, Utc::now()).into()
    }
}
