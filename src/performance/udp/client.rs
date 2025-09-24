use bytes::Bytes;
use chrono::Utc;
use colored::*;
use eyre::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::congestion::{BbrCongestionControl, CongestionControl};
use super::pacing::{PacedSend, Pacer};
use super::protocol::{
    ConnectionState, InFlightPacket, LossRecovery, StpPacket, calculate_rtt,
    current_timestamp_micros,
};
use crate::report::{TestReport, ThroughputMeasurement, ThroughputResult, UdpTestConfig};
use crate::utils::format::{format_bytes, format_throughput};

// TODO: Update all this client logic to match the TCP/HTTP implementations
// - Make test output download, upload, latency, separately, rather than a single upload measurement
// TODO: Need to support different modes based on test type

/// STP Client for bandwidth measurement
pub struct StpClient {
    socket: UdpSocket,
    connection: ConnectionState,
    congestion_control: Box<dyn CongestionControl + Send>,
    loss_recovery: LossRecovery,
    pacer: Pacer,

    // Statistics
    bytes_sent: u64,
    bytes_acked: u64,
    packets_sent: u64,
    packets_acked: u64,
    rtt_samples: Vec<Duration>,

    // Timestamps for tracking
    ack_timestamps: Arc<Mutex<HashMap<u64, u64>>>,
}

impl StpClient {
    pub async fn new(server_addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(server_addr).await?;

        let peer_addr = server_addr.parse()?;
        let connection = ConnectionState::new(peer_addr);
        let congestion_control = Box::new(BbrCongestionControl::new());
        let initial_rate = congestion_control.get_sending_rate();

        Ok(Self {
            socket,
            connection,
            congestion_control,
            loss_recovery: LossRecovery::new(),
            pacer: Pacer::new(initial_rate),
            bytes_sent: 0,
            bytes_acked: 0,
            packets_sent: 0,
            packets_acked: 0,
            rtt_samples: Vec::new(),
            ack_timestamps: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Send data with STP protocol
    pub async fn send_data(&mut self, payload: Bytes) -> Result<()> {
        let packet_number = self.connection.next_packet_number();
        let packet = StpPacket::new(
            packet_number,
            self.connection.last_received_packet,
            self.connection.last_received_timestamp,
            payload.clone(),
        );

        // Store timestamp for RTT calculation
        {
            let mut timestamps = self.ack_timestamps.lock();
            timestamps.insert(packet_number, packet.header.timestamp);
        }

        // Pace the sending
        let wait_duration = self.pacer.schedule_next_send(packet.payload.len() + 32);
        if let Some(duration) = wait_duration {
            PacedSend::new(Some(duration)).await;
        }

        // Send packet
        let encoded = packet.encode();
        self.socket.send(&encoded).await?;

        // Update statistics and congestion control
        let now = Instant::now();
        self.bytes_sent += encoded.len() as u64;
        self.packets_sent += 1;

        self.congestion_control.on_packet_sent(encoded.len(), now);
        self.pacer
            .update_rate(self.congestion_control.get_sending_rate());

        // Track in-flight packet
        let in_flight = InFlightPacket::new(packet_number, encoded.len(), encoded);
        self.loss_recovery.on_packet_sent(in_flight);

        Ok(())
    }

    /// Process received ACK packet
    pub async fn process_ack(&mut self, data: &[u8]) -> Result<()> {
        if let Some(packet) = StpPacket::decode(Bytes::copy_from_slice(data)) {
            self.connection.update_from_received(&packet.header);

            let now = Instant::now();

            // Calculate RTT if we have the timestamp
            let rtt = if packet.header.ack_timestamp_echo > 0 {
                calculate_rtt(packet.header.ack_timestamp_echo)
            } else {
                Duration::from_millis(1) // Default minimum RTT
            };

            // Process acknowledgment
            let (acked_packets, lost_packets) =
                self.loss_recovery.on_ack_received(packet.header.latest_ack);

            // Update statistics for acked packets
            for acked in &acked_packets {
                self.bytes_acked += acked.size as u64;
                self.packets_acked += 1;
                self.rtt_samples.push(rtt);

                // Notify congestion control
                self.congestion_control
                    .on_ack_received(acked.size, rtt, now);
            }

            // Handle lost packets
            for lost in &lost_packets {
                self.congestion_control.on_packet_lost(lost.size, now);

                // Retransmit lost packet with new packet number
                let new_packet_number = self.connection.next_packet_number();
                let retransmit_packet = StpPacket::decode(lost.data.clone()).map(|mut p| {
                    p.header.packet_number = new_packet_number;
                    p.header.timestamp = current_timestamp_micros();
                    p.header.latest_ack = self.connection.last_received_packet;
                    p.header.ack_timestamp_echo = self.connection.last_received_timestamp;
                    p
                });

                if let Some(packet) = retransmit_packet {
                    let encoded = packet.encode();
                    self.socket.send(&encoded).await?;

                    // Track retransmission
                    let mut retransmit_in_flight =
                        InFlightPacket::new(new_packet_number, encoded.len(), encoded);
                    retransmit_in_flight.retransmitted = true;
                    self.loss_recovery.on_packet_sent(retransmit_in_flight);
                }
            }

            // Update pacing rate
            self.pacer
                .update_rate(self.congestion_control.get_sending_rate());
        }

        Ok(())
    }

    /// Get current throughput statistics
    pub fn get_stats(&self) -> (u64, u64, u64, u64, f64, Duration) {
        let avg_rtt = if self.rtt_samples.is_empty() {
            Duration::from_millis(0)
        } else {
            let sum: Duration = self.rtt_samples.iter().sum();
            sum / self.rtt_samples.len() as u32
        };

        (
            self.bytes_sent,
            self.bytes_acked,
            self.packets_sent,
            self.packets_acked,
            self.congestion_control.get_sending_rate(),
            avg_rtt,
        )
    }
}

pub async fn run_udp_client(config: UdpTestConfig) -> Result<TestReport> {
    let server_addr = format!("{}:{}", config.server, config.port);
    let mut client = StpClient::new(&server_addr).await?;

    println!(
        "{}",
        format!(
            "Starting STP bandwidth test to server {}...",
            server_addr.cyan()
        )
        .green()
        .bold()
    );

    let test_duration = Duration::from_secs(config.duration);
    let payload_size = config.payload_sizes.first().copied().unwrap_or(1400);
    let payload = Bytes::from(vec![0u8; payload_size]);

    let start_time = Utc::now();
    let start = Instant::now();
    let mut last_report = start;

    // Create a buffer for receiving ACKs
    let mut recv_buffer = vec![0u8; 2048];

    // Main test loop
    let mut measurements = Vec::new();

    while start.elapsed() < test_duration {
        // Send data if congestion control allows
        let (bytes_sent, _, _, _, _sending_rate, _avg_rtt) = client.get_stats();
        let bytes_in_flight = bytes_sent - client.bytes_acked;

        if client.congestion_control.can_send(bytes_in_flight as usize)
            && let Err(e) = client.send_data(payload.clone()).await
        {
            eprintln!("Send error: {}", e);
            break;
        }

        // Try to receive ACKs (non-blocking)
        if let Ok(Ok((size, _))) = timeout(
            Duration::from_millis(1),
            client.socket.recv_from(&mut recv_buffer),
        )
        .await
            && let Err(e) = client.process_ack(&recv_buffer[..size]).await
        {
            eprintln!("ACK processing error: {}", e);
        }

        // Report progress and collect measurements
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed = start.elapsed();
            let (_bytes_sent, bytes_acked, packets_sent, packets_acked, sending_rate, avg_rtt) =
                client.get_stats();

            let packet_loss = if packets_sent > 0 {
                ((packets_sent - packets_acked) as f64 / packets_sent as f64) * 100.0
            } else {
                0.0
            };

            println!(
                "[{:3.0}s] {} sent/{} acked, {} transferred, {} rate, {:.1}ms RTT, {:.1}% loss",
                elapsed.as_secs_f64(),
                packets_sent,
                packets_acked,
                format_bytes(bytes_acked).yellow(),
                format_throughput(sending_rate / 125000.0).cyan(), // Convert to Mbps
                avg_rtt.as_secs_f64() * 1000.0,
                packet_loss
            );

            // Collect measurement
            let measurement_duration = last_report.elapsed();
            if measurement_duration > Duration::from_millis(500) {
                // Only collect if measurement is substantial
                measurements.push(ThroughputMeasurement::new(bytes_acked, elapsed));
            }

            last_report = Instant::now();
        }

        // Small delay to prevent busy waiting
        tokio::time::sleep(Duration::from_micros(100)).await;
    }

    // Final statistics
    let total_duration = start.elapsed();
    let (_bytes_sent, bytes_acked, packets_sent, packets_acked, _final_rate, avg_rtt) =
        client.get_stats();

    let final_mbps = (bytes_acked as f64 * 8.0) / (total_duration.as_secs_f64() * 1_000_000.0);
    let packet_loss = if packets_sent > 0 {
        ((packets_sent - packets_acked) as f64 / packets_sent as f64) * 100.0
    } else {
        0.0
    };

    println!(
        "\n{} Final Results: {} transferred, {:.2} Mbps, {:.1}ms avg RTT, {:.1}% packet loss",
        "STP Test Complete!".green().bold(),
        format_bytes(bytes_acked).yellow(),
        final_mbps,
        avg_rtt.as_secs_f64() * 1000.0,
        packet_loss
    );

    // Add final measurement if we don't have any
    if measurements.is_empty() {
        measurements.push(ThroughputMeasurement::new(bytes_acked, total_duration));
    }

    let result = ThroughputResult {
        measurements,
        total_duration,
        timestamp: chrono::Utc::now(),
    };

    Ok((start_time, config, result).into())
}
