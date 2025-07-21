use std::fmt::{self, Display, Formatter};

use colored::Colorize as _;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    report::{LatencyResult, ThroughputResult},
    utils::format::format_bytes,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestResult {
    pub latency: Option<LatencyResult>,
    /// Map of download results by payload size
    pub download: IndexMap<usize, ThroughputResult>,
    /// Map of upload results by payload size
    pub upload: IndexMap<usize, ThroughputResult>,
}

impl Display for HttpTestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Display latency if available
        if let Some(latency) = &self.latency {
            writeln!(f, "  {}", "Latency Results:".bright_green().bold())?;
            write!(f, "{latency}")?;
            writeln!(f)?;
        }

        // Display download results
        if !self.download.is_empty() {
            writeln!(f, "  {}", "Download Results:".bright_green().bold())?;
            for (size, result) in &self.download {
                writeln!(
                    f,
                    "    {} ({}):",
                    "Payload Size".bright_blue(),
                    format_bytes(*size).yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{result}");
                for line in result_str.lines() {
                    writeln!(f, "    {line}")?;
                }
            }
        }

        // Display upload results
        if !self.upload.is_empty() {
            writeln!(f, "  {}", "Upload Results:".bright_green().bold())?;
            for (size, result) in &self.upload {
                writeln!(
                    f,
                    "    {} ({}):",
                    "Payload Size".bright_blue(),
                    format_bytes(*size).yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{result}");
                for line in result_str.lines() {
                    writeln!(f, "    {line}")?;
                }
            }
        }

        Ok(())
    }
}
