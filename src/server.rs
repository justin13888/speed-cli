use anyhow::Result;
use tokio::net::{TcpListener, UdpSocket};
use tokio::io::AsyncReadExt;
use tokio::time::Duration;
use std::time::Instant;
use colored::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::network::*;

#[derive(Debug)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub port: u16,
}

pub async fn run_server(config: ServerConfig) -> Result<()> {
    let addr = format!("{}:{}", config.bind_addr, config.port);
    
    println!("Server listening on {}...", addr.cyan());
    
    // Start both TCP and UDP servers concurrently
    let tcp_handle = tokio::spawn(run_tcp_server(addr.clone()));
    let udp_handle = tokio::spawn(run_udp_server(addr));
    
    // Wait for either to complete (they should run indefinitely)
    tokio::select! {
        result = tcp_handle => {
            if let Err(e) = result? {
                eprintln!("TCP server error: {e}");
            }
        }
        result = udp_handle => {
            if let Err(e) = result? {
                eprintln!("UDP server error: {e}");
            }
        }
    }
    
    Ok(())
}

async fn run_tcp_server(addr: String) -> Result<()> {
    let listener = TcpListener::bind(&addr).await?;
    println!("{}", "TCP server ready to accept connections...".green());
    
    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("New TCP connection from {}", addr.to_string().cyan());
        
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 8192];
            let mut total_bytes = 0u64;
            let start_time = Instant::now();
            let mut last_report = Instant::now();
            
            loop {
                match socket.read(&mut buffer).await {
                    Ok(0) => {
                        // Connection closed
                        let duration = start_time.elapsed();
                        let bandwidth_mbps = (total_bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);
                        
                        println!(
                            "TCP connection from {} closed. {} received in {:.2}s ({})",
                            addr,
                            format_bytes(total_bytes).yellow(),
                            duration.as_secs_f64(),
                            format_bandwidth(bandwidth_mbps).green()
                        );
                        break;
                    }
                    Ok(n) => {
                        total_bytes += n as u64;
                        
                        // Report progress every 5 seconds
                        if last_report.elapsed() >= Duration::from_secs(5) {
                            let elapsed = start_time.elapsed();
                            let current_mbps = (total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);
                            
                            println!(
                                "TCP {}: {} received, {} bandwidth",
                                addr,
                                format_bytes(total_bytes).yellow(),
                                format_bandwidth(current_mbps).green()
                            );
                            
                            last_report = Instant::now();
                        }
                    }
                    Err(e) => {
                        eprintln!("TCP connection error from {addr}: {e}");
                        break;
                    }
                }
            }
        });
    }
}

async fn run_udp_server(addr: String) -> Result<()> {
    let socket = UdpSocket::bind(&addr).await?;
    println!("{}", "UDP server ready to receive packets...".green());
    
    let clients: Arc<Mutex<HashMap<std::net::SocketAddr, UdpClientState>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut buffer = vec![0u8; 2048];
    
    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((size, client_addr)) => {
                let clients = clients.clone();
                let data = buffer[..size].to_vec();
                
                tokio::spawn(async move {
                    handle_udp_packet(clients, client_addr, data).await;
                });
            }
            Err(e) => {
                eprintln!("UDP receive error: {e}");
            }
        }
    }
}

#[derive(Debug)]
struct UdpClientState {
    start_time: Instant,
    last_sequence: u32,
    total_packets: u32,
    total_bytes: u64,
    packets_received: u32,
    last_report: Instant,
}

impl UdpClientState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            last_sequence: 0,
            total_packets: 0,
            total_bytes: 0,
            packets_received: 0,
            last_report: now,
        }
    }
}

async fn handle_udp_packet(
    clients: Arc<Mutex<HashMap<std::net::SocketAddr, UdpClientState>>>,
    client_addr: std::net::SocketAddr,
    data: Vec<u8>,
) {
    let mut clients_map = clients.lock().await;
    let client_state = clients_map.entry(client_addr).or_insert_with(UdpClientState::new);
    
    // Check if this is a termination packet
    if data.len() >= 8 && data[0] == 0xFF && data[1] == 0xFF && data[2] == 0xFF && data[3] == 0xFF {
        let total_packets_sent = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let duration = client_state.start_time.elapsed();
        let bandwidth_mbps = (client_state.total_bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);
        let packet_loss = if total_packets_sent > 0 {
            ((total_packets_sent - client_state.packets_received) as f64 / total_packets_sent as f64) * 100.0
        } else {
            0.0
        };
        
        println!(
            "UDP session from {} completed: {} packets received/{} sent, {} received in {:.2}s ({}), {:.2}% packet loss",
            client_addr.to_string().cyan(),
            client_state.packets_received,
            total_packets_sent,
            format_bytes(client_state.total_bytes).yellow(),
            duration.as_secs_f64(),
            format_bandwidth(bandwidth_mbps).green(),
            packet_loss
        );
        
        clients_map.remove(&client_addr);
        return;
    }
    
    // Regular data packet
    if data.len() >= 4 {
        let sequence = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        client_state.last_sequence = sequence;
        client_state.packets_received += 1;
    }
    
    client_state.total_bytes += data.len() as u64;
    
    // Report progress every 5 seconds
    if client_state.last_report.elapsed() >= Duration::from_secs(5) {
        let elapsed = client_state.start_time.elapsed();
        let current_mbps = (client_state.total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);
        
        println!(
            "UDP {}: {} packets, {} received, {} bandwidth",
            client_addr,
            client_state.packets_received,
            format_bytes(client_state.total_bytes).yellow(),
            format_bandwidth(current_mbps).green()
        );
        
        client_state.last_report = Instant::now();
    }
}
