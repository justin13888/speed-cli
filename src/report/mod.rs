use core::time;

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
        }
    }
}

impl<C, R> From<(C, R)> for TestReport
where
    C: Into<TestConfig>,
    R: Into<TestResult>,
{
    fn from((config, result): (C, R)) -> Self {
        Self {
            config: config.into(),
            result: result.into(),
            timestamp: chrono::Utc::now(),
        }
    }
}
