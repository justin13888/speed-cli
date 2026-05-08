//! Composite report covering many per-protocol [`TestReport`]s
//! produced by `client suite`. Lives in a separate module so importers
//! and the renderer can treat it as a peer of `TestReport`.

use std::fmt::{self, Display, Formatter};

use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};

use crate::report::{REPORT_SCHEMA_VERSION, TestReport};
use crate::utils::env::Environment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteReport {
    pub schema_version: u32,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    /// speed-cli version that produced this suite.
    pub version: String,
    /// Server address the suite ran against (informational; each
    /// inner [`TestReport`]'s config holds the canonical value).
    pub server: String,
    pub environment: Option<Environment>,
    pub reports: Vec<NamedReport>,
    /// Phases that were skipped or failed. Useful for auditing why a
    /// suite report has no entry for, say, HTTP/3.
    #[serde(default)]
    pub skipped: Vec<SkippedPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedReport {
    pub label: String,
    pub report: TestReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedPhase {
    pub label: String,
    pub reason: String,
}

impl SuiteReport {
    pub fn new(server: String) -> Self {
        let now = Utc::now();
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            start_time: now,
            end_time: now,
            version: env!("CARGO_PKG_VERSION").to_string(),
            server,
            environment: Some(Environment::capture()),
            reports: Vec::new(),
            skipped: Vec::new(),
        }
    }

    pub fn record(&mut self, label: impl Into<String>, report: TestReport) {
        self.reports.push(NamedReport {
            label: label.into(),
            report,
        });
    }

    pub fn skip(&mut self, label: impl Into<String>, reason: impl Into<String>) {
        self.skipped.push(SkippedPhase {
            label: label.into(),
            reason: reason.into(),
        });
    }

    pub fn finalize(&mut self) {
        self.end_time = Utc::now();
    }
}

impl Display for SuiteReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}",
            "═══ Speed CLI Suite Report ═══".bright_cyan().bold()
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
            "Server".bright_white().bold(),
            self.server.cyan()
        )?;
        writeln!(
            f,
            "{}: {}",
            "Start".bright_white().bold(),
            self.start_time
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .yellow()
        )?;
        writeln!(
            f,
            "{}: {}",
            "End".bright_white().bold(),
            self.end_time
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
                .yellow()
        )?;

        if let Some(env) = &self.environment {
            writeln!(f)?;
            writeln!(f, "{}", "Environment:".bright_white().bold().underline())?;
            write!(f, "{env}")?;
        }
        writeln!(f)?;

        for nr in &self.reports {
            writeln!(
                f,
                "{}",
                format!("── Phase: {} ──", nr.label).bright_magenta().bold()
            )?;
            write!(f, "{}", nr.report)?;
            writeln!(f)?;
        }

        if !self.skipped.is_empty() {
            writeln!(f, "{}", "Skipped phases:".bright_yellow().bold())?;
            for s in &self.skipped {
                writeln!(f, "  {} — {}", s.label.yellow(), s.reason.red())?;
            }
        }
        Ok(())
    }
}
