use colored::*;
use eyre::Result;
use std::collections::VecDeque;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::Duration;

use crate::network::BandwidthMeasurement;
use crate::report::{SimpleTestResult, TcpTestConfig, TestReport};
use crate::utils::format::{format_bandwidth, format_bytes};

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    let addr = format!("{}:{}", config.server, config.port);
    let mut stream = TcpStream::connect(&addr).await?;

    println!("Connecting to server {}...", addr.cyan());
    println!("{}", "Connected! Starting TCP bandwidth test...".green());

    let test_duration = config.duration;
    let start_time = Instant::now();
    let mut total_bytes = 0u64;
    let mut measurements = VecDeque::new();

    // Send data continuously
    let data = vec![0u8; 8192]; // 8KB buffer
    let mut last_report = Instant::now();

    while start_time.elapsed() < test_duration {
        stream.write_all(&data).await?;
        total_bytes += data.len() as u64;

        // Report progress every second
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed = start_time.elapsed();
            let current_mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);

            println!(
                "[{:3.0}s] {} transferred, {} bandwidth",
                elapsed.as_secs_f64(),
                format_bytes(total_bytes).yellow(),
                format_bandwidth(current_mbps).green()
            );

            measurements.push_back(BandwidthMeasurement::new(total_bytes));
            if measurements.len() > 10 {
                measurements.pop_front();
            }

            last_report = Instant::now();
        }
    }

    let final_duration = start_time.elapsed();
    let result = SimpleTestResult::new(total_bytes, final_duration);

    println!("\n{}", "=== Test Results ===".bold().blue());
    println!(
        "Total bytes transferred: {}",
        format_bytes(result.bytes_transferred).yellow()
    );
    println!("Test duration: {:.2}s", result.duration.as_secs_f64());
    println!(
        "Average bandwidth: {}",
        format_bandwidth(result.bandwidth_mbps).green().bold()
    );

    Ok((config, result).into())
}
