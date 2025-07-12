use std::fmt::{self, Display, Formatter};
use std::{collections::HashMap, time::Duration};

use colored::Colorize as _;
use serde::{Deserialize, Serialize};

use crate::{
    TestType,
    performance::http::HttpVersion,
    report::{LatencyResult, ThroughputResult},
    utils::format::{format_bytes, format_throughput},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestResult {
    pub latency: Option<LatencyResult>,
    /// Map of download results by payload size
    pub download: HashMap<usize, ThroughputResult>,
    /// Map of upload results by payload size
    pub upload: HashMap<usize, ThroughputResult>,
    pub errors: Vec<String>,
}

// pub fn print_http_results(result: &HttpTestResult) {
//     println!("\n{}", "HTTP Speed Test Results".green().bold());
//     println!("{}", "=".repeat(50).green());

//     println!("Test Type: {}", result.test_type.cyan());
//     println!("HTTP Version: {}", result.http_version.cyan());
//     println!("Server: {}", result.server_url.cyan());
//     println!(
//         "Parallel Connections: {}",
//         result.parallel_connections.to_string().yellow()
//     );
//     println!("Test Duration: {:.2}s", result.test_duration.as_secs_f64());

//     if let Some(download) = result.download_mbps {
//         println!(
//             "Download Speed: {}",
//             format_throughput(download).green().bold()
//         );
//         println!(
//             "Data Downloaded: {}",
//             format_bytes(result.bytes_downloaded).yellow()
//         );
//     }

//     if let Some(upload) = result.upload_mbps {
//         println!("Upload Speed: {}", format_throughput(upload).green().bold());
//         println!(
//             "Data Uploaded: {}",
//             format_bytes(result.bytes_uploaded).yellow()
//         );
//     }

//     if let Some(latency) = result.latency_ms {
//         println!("Average Latency: {latency:.2} ms");
//         if let Some(jitter) = result.jitter_ms {
//             println!("Jitter: {jitter:.2} ms");
//         }
//     }

//     println!("DNS Resolution: {:.2} ms", result.dns_resolution_ms);

//     if !result.errors.is_empty() {
//         println!("\n{}:", "Errors".red().bold());
//         for error in &result.errors {
//             println!("  • {}", error.red());
//         }
//     }

//     println!();
// }

impl Display for HttpTestResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Display latency if available
        if let Some(latency) = &self.latency {
            writeln!(f, "  {}", "Latency Results:".bright_green().bold())?;
            write!(f, "{}", latency)?;
            writeln!(f)?;
        }

        // Display download results
        if !self.download.is_empty() {
            writeln!(f, "  {}", "Download Results:".bright_green().bold())?;
            for (size, result) in &self.download {
                writeln!(
                    f,
                    "    {} ({} bytes):",
                    "Payload Size".bright_blue(),
                    size.to_string().yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{}", result);
                for line in result_str.lines() {
                    writeln!(f, "    {}", line)?;
                }
            }
        }

        // Display upload results
        if !self.upload.is_empty() {
            writeln!(f, "  {}", "Upload Results:".bright_green().bold())?;
            for (size, result) in &self.upload {
                writeln!(
                    f,
                    "    {} ({} bytes):",
                    "Payload Size".bright_blue(),
                    size.to_string().yellow()
                )?;
                // Indent the throughput result output
                let result_str = format!("{}", result);
                for line in result_str.lines() {
                    writeln!(f, "    {}", line)?;
                }
            }
        }

        // Display errors if any
        if !self.errors.is_empty() {
            writeln!(f, "  {}:", "Errors".red().bold())?;
            for error in &self.errors {
                writeln!(f, "    • {}", error.red())?;
            }
        }

        Ok(())
    }
}
