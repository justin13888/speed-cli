use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

use crate::utils::env::Environment;

mod config;
mod errors;
mod identity;
mod measurement;
mod result;
mod suite;

pub use config::*;
pub use errors::*;
pub use identity::*;
pub use measurement::*;
pub use result::*;
pub use suite::*;

/// Current report schema version. Bump when any structural change
/// lands. With CBOR-only export and no backwards-compatibility, an
/// import whose `schema_version` does not match this constant is
/// rejected at import time.
///
/// All `*_us` fields in the schema are `u64` microseconds offset from
/// `TestReport.start_time` unless documented otherwise.
pub const REPORT_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    /// Schema version. Distinct from `version` (the binary that wrote the
    /// report). Imports verify this equals `REPORT_SCHEMA_VERSION`.
    pub schema_version: u32,
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
    /// Snapshot of the local environment when the test ran.
    pub environment: Environment,
    /// Identity and addresses of the two endpoints. The client side is
    /// always populated from local socket info; the server side is
    /// populated only when the protocol-level handshake succeeds (TCP
    /// `'H'` command, HTTP identity headers, UDP `Hello` packet).
    pub peers: Peers,
}

impl TestReport {
    pub fn new(
        start_time: DateTime<Utc>,
        config: TestConfig,
        result: TestResult,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            start_time,
            config,
            result,
            timestamp,
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: Environment::capture(),
            peers: Peers::local_only(),
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
            schema_version: REPORT_SCHEMA_VERSION,
            start_time: start_time.into(),
            config: config.into(),
            result: result.into(),
            timestamp: timestamp.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            environment: Environment::capture(),
            peers: Peers::local_only(),
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

        writeln!(f, "{}", "Environment:".bright_white().bold().underline())?;
        write!(f, "{}", self.environment)?;
        writeln!(f)?;

        writeln!(f, "{}", "Peers:".bright_white().bold().underline())?;
        write!(f, "{}", self.peers)?;
        writeln!(f)?;

        writeln!(f, "{}", "Configuration:".bright_white().bold().underline())?;
        write!(f, "{}", self.config)?;
        writeln!(f)?;

        writeln!(f, "{}", "Results:".bright_white().bold().underline())?;
        write!(f, "{}", self.result)?;

        Ok(())
    }
}
