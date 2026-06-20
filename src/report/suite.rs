//! Composite report covering many per-protocol [`TestReport`]s
//! produced by `client suite`. Lives in a separate module so importers
//! and the renderer can treat it as a peer of `TestReport`.

use std::fmt::{self, Display, Formatter};
use std::time::Duration;

use chrono::{DateTime, Utc};
use colored::*;
use serde::{Deserialize, Serialize};

use crate::TestType;
use crate::report::{REPORT_SCHEMA_VERSION, TestReport};
use crate::utils::env::Environment;
use crate::utils::format::format_bytes;

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
    /// Effective parameters this phase ran with. Recorded explicitly so
    /// the suite report can show, at a glance, what differs between
    /// phases and why — see [`PhaseParams`].
    pub params: PhaseParams,
    pub report: TestReport,
}

/// The parameters a single suite phase actually ran with. The suite
/// normalises these across protocols so results are comparable; any
/// genuine, intrinsic difference is captured in [`PhaseParams::deviations`]
/// rather than left implicit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseParams {
    /// Bulk payload per operation/request, in bytes. `None` for latency
    /// phases, which carry no bulk payload.
    pub payload_size: Option<usize>,
    /// I/O unit in bytes: the TCP/QUIC per-operation payload, the HTTP
    /// chunk size, or the UDP datagram size. Unified across protocols
    /// except where a protocol is intrinsically constrained.
    pub io_unit: usize,
    /// Parallel connections / streams.
    pub connections: usize,
    /// Wall-clock duration of the phase.
    pub duration: Duration,
    /// The test type actually executed. May differ from the phase label
    /// (the `*/full-duplex` HTTP rows run [`TestType::Simultaneous`],
    /// since HTTP cannot do true full-duplex).
    pub test_type: TestType,
    /// Intrinsic deviations from the suite baseline, e.g. UDP being
    /// single-stream. Empty when the phase matches the baseline exactly.
    #[serde(default)]
    pub deviations: Vec<String>,
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

    pub fn record(&mut self, label: impl Into<String>, params: PhaseParams, report: TestReport) {
        self.reports.push(NamedReport {
            label: label.into(),
            params,
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

impl Display for PhaseParams {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let payload = match self.payload_size {
            Some(n) => format_bytes(n),
            None => "n/a".to_string(),
        };
        writeln!(
            f,
            "  {}: payload={}  io-unit={}  connections={}  duration={}s  type={}",
            "params".bright_white().bold(),
            payload.yellow(),
            format_bytes(self.io_unit).yellow(),
            self.connections.to_string().yellow(),
            self.duration.as_secs().to_string().yellow(),
            self.test_type.to_string().yellow(),
        )?;
        for note in &self.deviations {
            writeln!(f, "  {} {}", "note:".bright_yellow().bold(), note.yellow())?;
        }
        Ok(())
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
            write!(f, "{}", nr.params)?;
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
