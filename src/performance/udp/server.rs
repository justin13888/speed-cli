use colored::*;
use eyre::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{ToSocketAddrs, UdpSocket};
use parking_lot::Mutex;
use tokio::time::Duration;
use bytes::Bytes;

use crate::utils::format::{format_bytes, format_throughput};
use super::protocol::{StpPacket, ConnectionState};

// TODO: Is parking_lot for Mutex

/// STP Server for bandwidth measurement  
pub struct StpServer {
    socket: UdpSocket,
    clients: Arc<Mutex<HashMap<std::net::SocketAddr, StpClientState>>>,
}

#[derive(Debug)]
struct StpClientState {
    connection: ConnectionState,
    start_time: Instant,
    total_bytes: u64,
    packets_received: u64,
    last_report: Instant,
    local_packet_number: u64,
    download_mode: bool,
    download_payload_size: usize,
}

impl StpClientState {
    fn new(client_addr: std::net::SocketAddr) -> Self {
        let now = Instant::now();
        Self {
            connection: ConnectionState::new(client_addr),
            start_time: now,
            total_bytes: 0,
            packets_received: 0,
            last_report: now,
            local_packet_number: 0,
            download_mode: false,
            download_payload_size: 1024,
        }
    }

    fn next_packet_number(&mut self) -> u64 {
        self.local_packet_number += 1;
        self.local_packet_number
    }
}

impl StpServer {
    pub async fn new(addr: impl ToSocketAddrs) -> Result<Self> {
        let socket = UdpSocket::bind(&addr).await?;
        Ok(Self {
            socket,
            clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn run(&self) -> Result<()> {
        println!("{}", "STP server ready to receive packets...".green());
        println!("UDP server listening on: {}", self.socket.local_addr()?);
        
        let mut buffer = vec![0u8; 2048];

        loop {
            match self.socket.recv_from(&mut buffer).await {
                Ok((size, client_addr)) => {
                    println!("Received {} bytes from {}", size, client_addr);
                    let clients = self.clients.clone();
                    let socket = &self.socket;
                    let data = Bytes::copy_from_slice(&buffer[..size]);

                    // Handle packet immediately (no need to spawn task for simple ACK)
                    if let Err(e) = self.handle_stp_packet(socket, clients, client_addr, data).await {
                        eprintln!("Error handling STP packet from {}: {}", client_addr, e);
                    }
                }
                Err(e) => {
                    eprintln!("STP receive error: {}", e);
                }
            }
        }
    }

    async fn handle_stp_packet(
        &self,
        socket: &UdpSocket,
        clients: Arc<Mutex<HashMap<std::net::SocketAddr, StpClientState>>>,
        client_addr: std::net::SocketAddr,
        data: Bytes,
    ) -> Result<()> {
        if let Some(packet) = StpPacket::decode(data) {
            let (ack_data, should_send_download_data, download_payload_size) = {
                let mut clients_map = clients.lock();
                let client_state = clients_map
                    .entry(client_addr)
                    .or_insert_with(|| StpClientState::new(client_addr));

                // Check if this is a download command
                if packet.payload.starts_with(b"DOWNLOAD") {
                    client_state.download_mode = true;
                    println!("Client {} requested download mode", client_addr.to_string().cyan());
                }
                
                // Check if this is a ping packet for latency measurement
                let is_ping = packet.payload.starts_with(b"PING");
                if is_ping {
                    println!("Client {} sent ping packet", client_addr.to_string().cyan());
                }

                // Update connection state
                client_state.connection.update_from_received(&packet.header);
                client_state.total_bytes += packet.payload.len() as u64;
                client_state.packets_received += 1;

                // Report progress periodically
                if client_state.last_report.elapsed() >= Duration::from_secs(2) {
                    let elapsed = client_state.start_time.elapsed();
                    let current_mbps = if elapsed.as_secs_f64() > 0.0 {
                        (client_state.total_bytes as f64 * 8.0) / (elapsed.as_secs_f64() * 1_000_000.0)
                    } else {
                        0.0
                    };

                    println!(
                        "STP {}: {} packets, {} received, {} throughput",
                        client_addr.to_string().cyan(),
                        client_state.packets_received,
                        format_bytes(client_state.total_bytes).yellow(),
                        format_throughput(current_mbps).green()
                    );

                    client_state.last_report = Instant::now();
                }

                // Handle connection teardown (empty payload could indicate end)
                if packet.payload.is_empty() && client_state.packets_received > 100 {
                    // This might be a termination signal
                    let duration = client_state.start_time.elapsed();
                    let final_mbps = if duration.as_secs_f64() > 0.0 {
                        (client_state.total_bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0)
                    } else {
                        0.0
                    };

                    println!(
                        "STP session from {} completed: {} packets received, {} total in {:.2}s ({})",
                        client_addr.to_string().cyan(),
                        client_state.packets_received,
                        format_bytes(client_state.total_bytes).yellow(),
                        duration.as_secs_f64(),
                        format_throughput(final_mbps).green()
                    );
                }

                // Prepare ACK
                let ack_packet_number = client_state.next_packet_number();
                let ack_packet = StpPacket::ack_only(
                    ack_packet_number,
                    packet.header.packet_number, // ACK this packet
                    packet.header.timestamp,     // Echo the timestamp
                );

                (ack_packet.encode(), client_state.download_mode, client_state.download_payload_size)
            }; // Lock is dropped here

            // Send ACK without holding the lock
            socket.send_to(&ack_data, client_addr).await?;

            // If in download mode, send download data packets
            if should_send_download_data {
                // Create download data packet
                let download_data = vec![0u8; download_payload_size];
                let payload = Bytes::from(download_data);
                
                // Get next packet number for download data
                let packet_number = {
                    let mut clients_map = clients.lock();
                    let client_state = clients_map.get_mut(&client_addr).unwrap();
                    client_state.next_packet_number()
                };
                
                let download_packet = StpPacket::new(
                    packet_number,
                    0, // No ACK needed for download data
                    0, // No timestamp echo
                    payload,
                );

                socket.send_to(&download_packet.encode(), client_addr).await?;
            }
        }

        Ok(())
    }
}

pub async fn run_udp_server(addr: impl ToSocketAddrs) -> Result<()> {
    let server = StpServer::new(addr).await?;
    server.run().await
}
