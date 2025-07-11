use colored::*;
use eyre::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::network::types::*;

pub async fn run_tcp_server(addr: impl ToSocketAddrs) -> Result<()> {
    let listener = TcpListener::bind(&addr).await?;
    println!("{}", "TCP server ready to accept connections...".green());

    let connection_id = Arc::new(AtomicU64::new(0));

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("New TCP connection from {}", addr.to_string().cyan());

        let connection_id = connection_id.fetch_add(1, Ordering::Relaxed);
        let handler = Arc::new(OptimizedTcpHandler::new(connection_id as usize));

        tokio::spawn({
            let handler = handler.clone();
            async move {
                let mut buffer = vec![0u8; 8192];

                loop {
                    match socket.read(&mut buffer).await {
                        Ok(0) => {
                            // Connection closed
                            let (total_bytes, duration, bandwidth_mbps) = handler.get_stats();

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
                            handler.add_bytes(n as u64);

                            // Report progress if necessary
                            if handler.should_report() {
                                let (total_bytes, _, bandwidth_mbps) = handler.get_stats();

                                println!(
                                    "TCP {}: {} received, {} bandwidth",
                                    addr,
                                    format_bytes(total_bytes).yellow(),
                                    format_bandwidth(bandwidth_mbps).green()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("TCP connection error from {addr}: {e}");
                            break;
                        }
                    }
                }
            }
        });
    }
}

pub async fn run_udp_server(addr: impl ToSocketAddrs) -> Result<()> {
    let socket = UdpSocket::bind(&addr).await?;
    println!("{}", "UDP server ready to receive packets...".green());

    let clients: Arc<Mutex<HashMap<std::net::SocketAddr, UdpClientState>>> =
        Arc::new(Mutex::new(HashMap::new()));
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
    let client_state = clients_map
        .entry(client_addr)
        .or_insert_with(UdpClientState::new);

    // Check if this is a termination packet
    if data.len() >= 8 && data[0] == 0xFF && data[1] == 0xFF && data[2] == 0xFF && data[3] == 0xFF {
        let total_packets_sent = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let duration = client_state.start_time.elapsed();
        let bandwidth_mbps =
            (client_state.total_bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);
        let packet_loss = if total_packets_sent > 0 {
            ((total_packets_sent - client_state.packets_received) as f64
                / total_packets_sent as f64)
                * 100.0
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
        let current_mbps =
            (client_state.total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0);

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

// Optimized connection state to reduce allocations
#[derive(Debug)]
struct OptimizedTcpHandler {
    total_bytes: AtomicU64,
    start_time: Instant,
    last_report: std::sync::Mutex<Instant>,
    connection_id: usize,
}

impl OptimizedTcpHandler {
    fn new(connection_id: usize) -> Self {
        let now = Instant::now();
        Self {
            total_bytes: AtomicU64::new(0),
            start_time: now,
            last_report: std::sync::Mutex::new(now),
            connection_id,
        }
    }

    fn add_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn should_report(&self) -> bool {
        let mut last_report = self.last_report.lock().unwrap();
        if last_report.elapsed() >= Duration::from_secs(5) {
            *last_report = Instant::now();
            true
        } else {
            false
        }
    }

    fn get_stats(&self) -> (u64, Duration, f64) {
        let bytes = self.total_bytes.load(Ordering::Relaxed);
        let duration = self.start_time.elapsed();
        let bandwidth_mbps = (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);
        (bytes, duration, bandwidth_mbps)
    }
}
