use eyre::Result;
use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, Duration};
use std::time::Instant;
use colored::*;
use std::collections::VecDeque;

use crate::network::*;

#[derive(Debug)]
pub struct ClientConfig {
    pub server_addr: String,
    pub port: u16,
    pub duration: u64,
    pub use_udp: bool,
    pub target_bandwidth: f64,
    pub export_file: Option<String>,
}

pub async fn run_client(config: ClientConfig) -> Result<()> {
    let addr = format!("{}:{}", config.server_addr, config.port);
    
    println!("Connecting to server {}...", addr.cyan());
    
    if config.use_udp {
        run_udp_client(config).await
    } else {
        run_tcp_client(config).await
    }
}

async fn run_tcp_client(config: ClientConfig) -> Result<()> {
    let addr = format!("{}:{}", config.server_addr, config.port);
    let mut stream = TcpStream::connect(&addr).await?;
    
    println!("{}", "Connected! Starting TCP bandwidth test...".green());
    
    let test_duration = Duration::from_secs(config.duration);
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
    let result = TestResult::new(total_bytes, final_duration);
    
    println!("\n{}", "=== Test Results ===".bold().blue());
    println!("Total bytes transferred: {}", format_bytes(result.bytes_transferred).yellow());
    println!("Test duration: {:.2}s", result.duration.as_secs_f64());
    println!("Average bandwidth: {}", format_bandwidth(result.bandwidth_mbps).green().bold());
    
    if let Some(export_file) = config.export_file {
        crate::export::export_results(&[result], &export_file).await?;
        println!("Results exported to: {}", export_file.cyan());
    }
    
    Ok(())
}

async fn run_udp_client(config: ClientConfig) -> Result<()> {
    use tokio::net::UdpSocket;
    
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let server_addr = format!("{}:{}", config.server_addr, config.port);
    socket.connect(&server_addr).await?;
    
    println!("{}", "Connected! Starting UDP bandwidth test...".green());
    
    let test_duration = Duration::from_secs(config.duration);
    let start_time = Instant::now();
    let mut total_bytes = 0u64;
    let mut packet_count = 0u32;
    
    // Calculate packet size and interval for target bandwidth
    let packet_size = 1024; // 1KB packets
    let target_bps = config.target_bandwidth * 1_000_000.0; // Convert Mbps to bps
    let packets_per_second = target_bps / (packet_size as f64 * 8.0);
    let packet_interval = Duration::from_secs_f64(1.0 / packets_per_second);
    
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
    let result = TestResult::new(total_bytes, final_duration);
    
    println!("\n{}", "=== Test Results ===".bold().blue());
    println!("Total packets sent: {packet_count}");
    println!("Total bytes transferred: {}", format_bytes(result.bytes_transferred).yellow());
    println!("Test duration: {:.2}s", result.duration.as_secs_f64());
    println!("Average bandwidth: {}", format_bandwidth(result.bandwidth_mbps).green().bold());
    
    if let Some(export_file) = config.export_file {
        crate::export::export_results(&[result], &export_file).await?;
        println!("Results exported to: {}", export_file.cyan());
    }
    
    Ok(())
}
