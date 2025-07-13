use chrono::Utc;
use colored::*;
use eyre::Result;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::time::{Duration, sleep};

use crate::report::{TestReport, ThroughputMeasurement, ThroughputResult, UdpTestConfig};
use crate::utils::format::{format_bytes, format_throughput};

pub async fn run_udp_client(config: UdpTestConfig) -> Result<TestReport> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let server_addr = format!("{}:{}", config.server, config.port);
    socket.connect(&server_addr).await?;

    println!(
        "{}",
        format!(
            "Starting UDP throughput test to server {}...",
            server_addr.cyan()
        )
        .green()
        .bold()
    );

    let test_duration = Duration::from_secs(config.duration);
    let mut total_bytes = 0u64;
    let mut packet_count = 0u32;

    // Calculate packet size and interval for target throughput
    let packet_size = 1024; // 1KB packets
    // TODO: You're supposed to implement dynamic packet sizing to maximize throughput while avoiding overwhelming network (e.g. AIMD CUBIC)
    let packet_interval = Duration::from_secs_f64(1.0 / 100.0); // 100 packets per second

    let mut data = vec![0u8; packet_size];
    let start_time = Utc::now();

    let start = Instant::now();
    let mut last_report = start;
    let mut next_packet_time = Instant::now();

    while start.elapsed() < test_duration {
        // Add sequence number to packet
        data[0..4].copy_from_slice(&packet_count.to_be_bytes());

        socket.send(&data).await?;
        total_bytes += data.len() as u64;
        packet_count += 1;

        // Report progress every second
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed = start.elapsed();
            let current_mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);

            println!(
                "[{:3.0}s] {} packets, {} transferred, {} throughput",
                elapsed.as_secs_f64(),
                packet_count,
                format_bytes(total_bytes).yellow(),
                format_throughput(current_mbps).green()
            );

            last_report = Instant::now();
        }

        // Wait for next packet time to maintain target throughput
        next_packet_time += packet_interval;
        if next_packet_time > Instant::now() {
            sleep(next_packet_time - Instant::now()).await;
        }
    }

    // Send termination packet
    let mut end_packet = vec![0xFFu8; 8];
    end_packet[4..8].copy_from_slice(&packet_count.to_be_bytes());
    socket.send(&end_packet).await?;

    let total_duration = start.elapsed();
    let result = ThroughputResult {
        measurements: vec![ThroughputMeasurement::new(total_bytes, total_duration)], // TODO: Collect more measurements along the way instead. Also requires server-side support
        total_duration,
        timestamp: chrono::Utc::now(),
    };

    // println!("\n{}", "=== Test Results ===".bold().blue());
    // println!("{result:#?}");

    Ok((start_time, config, result).into())
}
