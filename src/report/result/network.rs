use std::fmt::{self, Display, Formatter};

use colored::Colorize as _;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    report::{LatencyResult, ThroughputResult},
    utils::format::format_bytes,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkTestResult {
    pub latency: Option<LatencyResult>,
    /// Map of download results by payload size
    pub download: IndexMap<usize, ThroughputResult>,
    /// Map of upload results by payload size
    pub upload: IndexMap<usize, ThroughputResult>,
    /// Protocol type for display purposes
    pub protocol: NetworkProtocol,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NetworkProtocol {
    Http,
    Tcp,
}

impl NetworkTestResult {
    pub fn new_http() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Http,
        }
    }

    pub fn new_tcp() -> Self {
        Self {
            latency: None,
            download: IndexMap::new(),
            upload: IndexMap::new(),
            protocol: NetworkProtocol::Tcp,
        }
    }
}

impl Display for NetworkTestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let protocol_prefix = match self.protocol {
            NetworkProtocol::Http => "HTTP ",
            NetworkProtocol::Tcp => "TCP ",
        };

        // Display latency if available
        if let Some(latency) = &self.latency {
            writeln!(
                f,
                "  {}",
                format!("{}Latency Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
            write!(f, "{latency}")?;
            writeln!(f)?;
        }

        // Display download results
        if !self.download.is_empty() {
            writeln!(
                f,
                "  {}",
                format!("{}Download Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
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
            writeln!(
                f,
                "  {}",
                format!("{}Upload Results:", protocol_prefix)
                    .bright_green()
                    .bold()
            )?;
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
