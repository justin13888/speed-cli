use colored::*;
use eyre::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::ToSocketAddrs;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::utils::format::{format_bytes, format_throughput};

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
        let throughput_mbps =
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
            format_throughput(throughput_mbps).green(),
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
            "UDP {}: {} packets, {} received, {} throughput",
            client_addr,
            client_state.packets_received,
            format_bytes(client_state.total_bytes).yellow(),
            format_throughput(current_mbps).green()
        );

        client_state.last_report = Instant::now();
    }
}
