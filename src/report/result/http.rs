use std::time::Duration;

use colored::Colorize as _;
use serde::{Deserialize, Serialize};

use crate::utils::format::{format_bandwidth, format_bytes};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTestResult {
    pub test_type: String,
    pub http_version: String,
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub latency_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub connection_time_ms: f64,
    pub ssl_handshake_ms: Option<f64>,
    pub dns_resolution_ms: f64,
    pub parallel_connections: usize,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub test_duration: Duration,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub server_url: String,
    pub errors: Vec<String>,
}

pub fn print_http_results(result: &HttpTestResult) {
    println!("\n{}", "HTTP Speed Test Results".green().bold());
    println!("{}", "=".repeat(50).green());

    println!("Test Type: {}", result.test_type.cyan());
    println!("HTTP Version: {}", result.http_version.cyan());
    println!("Server: {}", result.server_url.cyan());
    println!(
        "Parallel Connections: {}",
        result.parallel_connections.to_string().yellow()
    );
    println!("Test Duration: {:.2}s", result.test_duration.as_secs_f64());

    if let Some(download) = result.download_mbps {
        println!(
            "Download Speed: {}",
            format_bandwidth(download).green().bold()
        );
        println!(
            "Data Downloaded: {}",
            format_bytes(result.bytes_downloaded).yellow()
        );
    }

    if let Some(upload) = result.upload_mbps {
        println!("Upload Speed: {}", format_bandwidth(upload).green().bold());
        println!(
            "Data Uploaded: {}",
            format_bytes(result.bytes_uploaded).yellow()
        );
    }

    if let Some(latency) = result.latency_ms {
        println!("Average Latency: {latency:.2} ms");
        if let Some(jitter) = result.jitter_ms {
            println!("Jitter: {jitter:.2} ms");
        }
    }

    println!("DNS Resolution: {:.2} ms", result.dns_resolution_ms);

    if !result.errors.is_empty() {
        println!("\n{}:", "Errors".red().bold());
        for error in &result.errors {
            println!("  • {}", error.red());
        }
    }

    println!();
}
