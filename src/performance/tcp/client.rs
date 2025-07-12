use chrono::Utc;
use colored::*;
use eyre::Result;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::Duration;

use crate::report::{ThroughputMeasurement, ThroughputResult, TcpTestConfig, TestReport};
use crate::utils::format::{format_throughput, format_bytes};

pub async fn run_tcp_client(config: TcpTestConfig) -> Result<TestReport> {
    let addr = format!("{}:{}", config.server, config.port);
    let mut stream = TcpStream::connect(&addr).await?;

    println!("Connecting to server {}...", addr.cyan());
    println!("{}", "Connected! Starting TCP throughput test...".green());

    let test_duration = config.duration;
    let mut measurements = vec![];
    let mut total_bytes = 0usize;

    // Send data continuously
    let data = vec![0u8; 8192]; // 8KB buffer

    let start_time = Utc::now();
    let start = Instant::now();
    let mut last_report = start;

    while start.elapsed() < test_duration {
        let send_start = Instant::now();
        stream.write_all(&data).await?;
        let send_duration = send_start.elapsed();
        total_bytes += data.len();
        measurements.push(ThroughputMeasurement::new(data.len() as u64, send_duration));

        // Report progress every second
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed = start.elapsed();
            let current_mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);

            println!(
                "[{:3.0}s] {} transferred, {} throughput",
                elapsed.as_secs_f64(),
                format_bytes(total_bytes).yellow(),
                format_throughput(current_mbps).green()
            );

            last_report = Instant::now();
        }
    }

    let total_duration = start.elapsed();
    let result = ThroughputResult {
        measurements,
        total_duration,
        timestamp: chrono::Utc::now(),
    }; // TODO: Review

    // println!("\n{}", "=== Test Results ===".bold().blue());
    // println!("{result:#?}");

    Ok((start_time, config, result).into())
}
