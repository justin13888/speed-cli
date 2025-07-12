use colored::*;
use eyre::Result;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::time::{Duration, sleep};

use crate::report::{SimpleTestResult, TestReport, UdpTestConfig};
use crate::utils::format::{format_bandwidth, format_bytes};

pub async fn run_udp_client(config: UdpTestConfig) -> Result<TestReport> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let server_addr = format!("{}:{}", config.server, config.port);
    socket.connect(&server_addr).await?;

    println!("Connecting to server {}...", server_addr.cyan());
    println!("{}", "Connected! Starting UDP bandwidth test...".green());

    let test_duration = Duration::from_secs(config.duration);
    let start_time = Instant::now();
    let mut total_bytes = 0u64;
    let mut packet_count = 0u32;

    // Calculate packet size and interval for target bandwidth
    let packet_size = 1024; // 1KB packets
    // TODO: You're supposed to implement dynamic packet sizing to maximize throughput while avoiding overwhelming network (e.g. AIMD CUBIC)
    let packet_interval = Duration::from_secs_f64(1.0 / 100.0); // 100 packets per second

    let mut data = vec![0u8; packet_size];
    let mut last_report = Instant::now();
    let mut next_packet_time = Instant::now();

    while start_time.elapsed() < test_duration {
        // Add sequence number to packet
        data[0..4].copy_from_slice(&packet_count.to_be_bytes());

        socket.send(&data).await?;
        total_bytes += data.len() as u64;
        packet_count += 1;

        // Report progress every second
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed = start_time.elapsed();
            let current_mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);

            println!(
                "[{:3.0}s] {} packets, {} transferred, {} bandwidth",
                elapsed.as_secs_f64(),
                packet_count,
                format_bytes(total_bytes).yellow(),
                format_bandwidth(current_mbps).green()
            );

            last_report = Instant::now();
        }

        // Wait for next packet time to maintain target bandwidth
        next_packet_time += packet_interval;
        if next_packet_time > Instant::now() {
            sleep(next_packet_time - Instant::now()).await;
        }
    }

    // Send termination packet
    let mut end_packet = vec![0xFFu8; 8];
    end_packet[4..8].copy_from_slice(&packet_count.to_be_bytes());
    socket.send(&end_packet).await?;

    let final_duration = start_time.elapsed();
    let result = SimpleTestResult::new(total_bytes, final_duration);

    println!("\n{}", "=== Test Results ===".bold().blue());
    println!("Total packets sent: {packet_count}");
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
