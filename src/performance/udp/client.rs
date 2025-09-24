use bytes::Bytes;
use chrono::Utc;
use colored::Colorize as _;
use eyre::Result;
use parking_lot::Mutex;
use rand::RngCore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};
use tracing::trace;

use super::congestion::{BbrCongestionControl, CongestionControl};
use super::pacing::{PacedSend, Pacer};
use super::protocol::{
    ConnectionState, InFlightPacket, LossRecovery, StpPacket, calculate_rtt,
    current_timestamp_micros,
};
use crate::{
    TestType,
    report::{
        ConnectionError, LatencyMeasurement, LatencyResult, NetworkTestResult, TestReport,
        ThroughputMeasurement, ThroughputResult, UdpTestConfig,
    },
    utils::{
        format::format_bytes,
        instrumentation::{
            LatencyStatsCollector, ProgressBarType, ThroughputStatsCollector, create_progress_bar,
        },
    },
};

// TODO: Verify upload, download, latency modes all work correctly
// TODO: Improve the STP implementation performance

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

    println!(
        "{}",
        format!("Starting UDP test to server {}...", server_addr.cyan())
            .green()
            .bold()
    );

    let start_time = Utc::now();

    let mut result = NetworkTestResult::new_udp();

    match config.test_type {
        TestType::LatencyOnly => {
            result.latency = measure_udp_latency(&config).await?;
        }
        TestType::Download => {
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    )
                    .await?,
                );
            }
        }
        TestType::Upload => {
            for payload_size in &config.payload_sizes {
                result.upload.insert(
                    *payload_size,
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    )
                    .await?,
                );
            }
        }
        TestType::Bidirectional => {
            // Run download and upload sequentially
            for payload_size in &config.payload_sizes {
                result.download.insert(
                    *payload_size,
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    )
                    .await?,
                );
                result.upload.insert(
                    *payload_size,
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    )
                    .await?,
                );
            }
        }
        TestType::Simultaneous => {
            // Run download and upload concurrently
            for payload_size in &config.payload_sizes {
                let (download_result, upload_result) = tokio::join!(
                    run_download_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    ),
                    run_upload_test(
                        &config.server,
                        config.port,
                        config.parallel_streams,
                        *payload_size,
                        Duration::from_secs(config.duration),
                    )
                );

                result.download.insert(*payload_size, download_result?);
                result.upload.insert(*payload_size, upload_result?);
            }
        }
    }

    Ok((start_time, config, result).into())
}

/// Measure UDP latency using simple UDP packets
async fn measure_udp_latency(config: &UdpTestConfig) -> Result<Option<LatencyResult>> {
    let addr = format!("{}:{}", config.server, config.port);
    let duration = Duration::from_secs(config.duration);
    let mut measurements = Vec::new();

    println!("Measuring UDP latency for {duration:?}...");

    // Create progress bar for latency measurement
    let progress_bar = create_progress_bar(ProgressBarType::Latency, duration);

    let start = Instant::now();

    // Set up instrumentation
    let (stats_collector, tx) = LatencyStatsCollector::new(progress_bar.clone(), start, duration);

    while start.elapsed() < duration {
        let connect_start = Instant::now();

        // Create STP client for latency measurement
        let mut client = match StpClient::new(&addr).await {
            Ok(c) => c,
            Err(_) => {
                let measurement = LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());
                let _ = tx.send(measurement);
                continue;
            }
        };

        // Send an STP ping packet
        let ping_packet = StpPacket::new(
            client.connection.next_packet_number(),
            0,
            0,
            Bytes::from("PING"),
        );

        match client.socket.send(&ping_packet.encode()).await {
            Ok(_) => {
                // Try to receive a response (with timeout)
                let mut buffer = [0u8; 2048];
                match timeout(Duration::from_millis(1000), client.socket.recv(&mut buffer)).await {
                    Ok(Ok(size)) => {
                        if let Some(_response_packet) =
                            StpPacket::decode(Bytes::copy_from_slice(&buffer[..size]))
                        {
                            let rtt = connect_start.elapsed().as_secs_f64() * 1000.0;
                            let measurement = LatencyMeasurement {
                                rtt_ms: Some(rtt),
                                elapsed_time: start.elapsed(),
                            };
                            measurements.push(measurement.clone());

                            // Send to stats collector (non-blocking)
                            let _ = tx.send(measurement);
                        } else {
                            let measurement = LatencyMeasurement {
                                rtt_ms: None,
                                elapsed_time: start.elapsed(),
                            };
                            measurements.push(measurement.clone());

                            // Send to stats collector (non-blocking)
                            let _ = tx.send(measurement);
                        }
                    }
                    _ => {
                        let measurement = LatencyMeasurement {
                            rtt_ms: None,
                            elapsed_time: start.elapsed(),
                        };
                        measurements.push(measurement.clone());

                        // Send to stats collector (non-blocking)
                        let _ = tx.send(measurement);
                    }
                }
            }
            Err(e) => {
                let measurement = LatencyMeasurement {
                    rtt_ms: None,
                    elapsed_time: start.elapsed(),
                };
                measurements.push(measurement.clone());

                // Send to stats collector (non-blocking)
                let _ = tx.send(measurement);

                trace!("UDP send error while measuring latency: {e}");
            }
        }

        // Wait between packets to avoid overwhelming the server
        sleep(Duration::from_millis(100)).await;
    }

    // Drop the sender to signal stats collector to finish
    drop(tx);

    // Wait for stats collector to complete and get measurements
    measurements = stats_collector
        .finish(progress_bar, "Latency measurement complete".to_string())
        .await;

    if measurements.is_empty() {
        return Ok(None);
    }

    Ok(Some(LatencyResult {
        measurements,
        timestamp: chrono::Utc::now(),
    }))
}

async fn run_download_test(
    server: &str,
    port: u16,
    _parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting UDP download test with {} payload size...",
        format_bytes(payload_size).yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Download, duration);

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Set up instrumentation
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let addr = format!("{server}:{port}");
    let mut client = StpClient::new(&addr).await?;

    // Send a download command to the server first
    let download_cmd = StpPacket::new(
        client.connection.next_packet_number(),
        0,
        0,
        Bytes::from("DOWNLOAD"),
    );
    let encoded = download_cmd.encode();
    println!(
        "Sending DOWNLOAD command to server... (size: {} bytes)",
        encoded.len()
    );
    client.socket.send(&encoded).await?;
    println!("DOWNLOAD command sent, waiting for response...");

    let mut recv_buffer = vec![0u8; 2048];

    while start_time.elapsed() < duration {
        // Try to receive data (non-blocking with short timeout)
        match timeout(
            Duration::from_millis(10),
            client.socket.recv(&mut recv_buffer),
        )
        .await
        {
            Ok(Ok(size)) => {
                let read_start = Instant::now();

                // Process received packet
                if let Some(packet) =
                    StpPacket::decode(Bytes::copy_from_slice(&recv_buffer[..size]))
                {
                    // Send ACK
                    let ack_packet = StpPacket::ack_only(
                        client.connection.next_packet_number(),
                        packet.header.packet_number,
                        packet.header.timestamp,
                    );
                    let _ = client.socket.send(&ack_packet.encode()).await;

                    let measurement = ThroughputMeasurement::new(
                        packet.payload.len() as u64,
                        read_start.elapsed(),
                    );
                    measurements.push(measurement.clone());
                    let _ = tx.send(measurement);
                }
            }
            _ => {
                // No data received, continue waiting
                continue;
            }
        }

        // Small delay to prevent busy waiting
        tokio::time::sleep(Duration::from_micros(100)).await;
    }

    // Drop the sender to signal stats collector to finish
    drop(tx);

    // Wait for stats collector to complete and get measurements
    measurements = stats_collector
        .finish(progress_bar, "Download complete".to_string())
        .await;

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}

async fn run_upload_test(
    server: &str,
    port: u16,
    _parallel_connections: usize,
    payload_size: usize,
    duration: Duration,
) -> Result<ThroughputResult> {
    println!(
        "Starting UDP upload test with {} payload size...",
        format_bytes(payload_size).yellow()
    );

    // Create progress bar
    let progress_bar = create_progress_bar(ProgressBarType::Upload, duration);

    let mut measurements = Vec::new();
    let start_time = Instant::now();

    // Generate upload data
    let mut upload_data = vec![0u8; payload_size];
    rand::rng().fill_bytes(&mut upload_data);
    let payload = Bytes::from(upload_data);

    // Set up instrumentation
    let (stats_collector, tx) =
        ThroughputStatsCollector::new(progress_bar.clone(), start_time, duration);

    let addr = format!("{server}:{port}");
    let mut client = StpClient::new(&addr).await?;

    let mut recv_buffer = vec![0u8; 2048];

    while start_time.elapsed() < duration {
        // Send data if congestion control allows
        let (bytes_sent, _, _, _, _sending_rate, _avg_rtt) = client.get_stats();
        let bytes_in_flight = bytes_sent - client.bytes_acked;

        if client.congestion_control.can_send(bytes_in_flight as usize) {
            let write_start = Instant::now();
            match client.send_data(payload.clone()).await {
                Ok(_) => {
                    let measurement =
                        ThroughputMeasurement::new(payload.len() as u64, write_start.elapsed());
                    measurements.push(measurement.clone());

                    // Send to stats collector (non-blocking)
                    let _ = tx.send(measurement);
                }
                Err(e) => {
                    let measurement = ThroughputMeasurement::new_error(
                        ConnectionError::Unknown(format!("UDP send error: {e}")),
                        write_start.elapsed(),
                        0,
                    );
                    measurements.push(measurement.clone());
                    let _ = tx.send(measurement);
                    break;
                }
            }
        }

        // Try to receive ACKs (non-blocking)
        if let Ok(Ok((size, _))) = timeout(
            Duration::from_millis(1),
            client.socket.recv_from(&mut recv_buffer),
        )
        .await
        {
            let _ = client.process_ack(&recv_buffer[..size]).await;
        }

        // Small delay to prevent busy waiting
        tokio::time::sleep(Duration::from_micros(100)).await;
    }

    // Drop the sender to signal stats collector to finish
    drop(tx);

    // Wait for stats collector to complete and get measurements
    measurements = stats_collector
        .finish(progress_bar, "Upload complete".to_string())
        .await;

    let end_time = Instant::now();

    Ok(ThroughputResult {
        measurements,
        total_duration: end_time.duration_since(start_time),
        timestamp: chrono::Utc::now(),
    })
}
