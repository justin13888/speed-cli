use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// STP (Simple Transport Protocol) packet header
/// Fixed 32-byte header for all packets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StpHeader {
    /// Unique, monotonically increasing 64-bit packet number
    pub packet_number: u64,
    /// 64-bit microsecond timestamp set by sender
    pub timestamp: u64,
    /// Highest packet number received from peer
    pub latest_ack: u64,
    /// Echo of timestamp from acknowledged packet
    pub ack_timestamp_echo: u64,
}

impl StpHeader {
    pub const SIZE: usize = 32;

    pub fn new(packet_number: u64, latest_ack: u64, ack_timestamp_echo: u64) -> Self {
        Self {
            packet_number,
            timestamp: current_timestamp_micros(),
            latest_ack,
            ack_timestamp_echo,
        }
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u64(self.packet_number);
        buf.put_u64(self.timestamp);
        buf.put_u64(self.latest_ack);
        buf.put_u64(self.ack_timestamp_echo);
    }

    pub fn decode(mut buf: Bytes) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }

        Some(Self {
            packet_number: buf.get_u64(),
            timestamp: buf.get_u64(),
            latest_ack: buf.get_u64(),
            ack_timestamp_echo: buf.get_u64(),
        })
    }
}

/// Complete STP packet
#[derive(Debug, Clone)]
pub struct StpPacket {
    pub header: StpHeader,
    pub payload: Bytes,
}

impl StpPacket {
    pub fn new(
        packet_number: u64,
        latest_ack: u64,
        ack_timestamp_echo: u64,
        payload: Bytes,
    ) -> Self {
        Self {
            header: StpHeader::new(packet_number, latest_ack, ack_timestamp_echo),
            payload,
        }
    }

    pub fn ack_only(packet_number: u64, latest_ack: u64, ack_timestamp_echo: u64) -> Self {
        Self {
            header: StpHeader::new(packet_number, latest_ack, ack_timestamp_echo),
            payload: Bytes::new(),
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(StpHeader::SIZE + self.payload.len());
        self.header.encode(&mut buf);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn decode(data: Bytes) -> Option<Self> {
        if data.len() < StpHeader::SIZE {
            return None;
        }

        let header = StpHeader::decode(data.slice(0..StpHeader::SIZE))?;
        let payload = data.slice(StpHeader::SIZE..);

        Some(Self { header, payload })
    }

    pub fn is_ack_only(&self) -> bool {
        self.payload.is_empty()
    }
}

/// Connection state for tracking peer
#[derive(Debug)]
pub struct ConnectionState {
    pub peer_addr: SocketAddr,
    pub local_packet_number: u64,
    pub peer_latest_ack: u64,
    pub last_received_packet: u64,
    pub last_received_timestamp: u64,
    pub established: bool,
}

impl ConnectionState {
    pub fn new(peer_addr: SocketAddr) -> Self {
        Self {
            peer_addr,
            local_packet_number: 0,
            peer_latest_ack: 0,
            last_received_packet: 0,
            last_received_timestamp: 0,
            established: false,
        }
    }

    pub fn next_packet_number(&mut self) -> u64 {
        self.local_packet_number += 1;
        self.local_packet_number
    }

    pub fn update_from_received(&mut self, header: &StpHeader) {
        self.peer_latest_ack = header.latest_ack;
        self.last_received_packet = header.packet_number;
        self.last_received_timestamp = header.timestamp;

        if !self.established {
            self.established = true;
        }
    }
}

/// Packet loss detection and retransmission tracking
#[derive(Debug, Clone)]
pub struct InFlightPacket {
    pub packet_number: u64,
    pub sent_time: u64,
    pub size: usize,
    pub data: Bytes,
    pub retransmitted: bool,
}

impl InFlightPacket {
    pub fn new(packet_number: u64, size: usize, data: Bytes) -> Self {
        Self {
            packet_number,
            sent_time: current_timestamp_micros(),
            size,
            data,
            retransmitted: false,
        }
    }
}

/// Loss recovery state
#[derive(Debug)]
pub struct LossRecovery {
    pub in_flight: VecDeque<InFlightPacket>,
    pub largest_acked: u64,
    pub loss_threshold: u64,
    pub loss_timeout: Duration,
}

impl LossRecovery {
    pub fn new() -> Self {
        Self {
            in_flight: VecDeque::new(),
            largest_acked: 0,
            loss_threshold: 3, // Declare lost after 3 higher packets are acked
            loss_timeout: Duration::from_millis(1000), // 1 second timeout
        }
    }

    pub fn on_packet_sent(&mut self, packet: InFlightPacket) {
        self.in_flight.push_back(packet);
    }

    pub fn on_ack_received(
        &mut self,
        acked_packet: u64,
    ) -> (Vec<InFlightPacket>, Vec<InFlightPacket>) {
        let mut acked_packets = Vec::new();
        let mut lost_packets = Vec::new();
        let current_time = current_timestamp_micros();

        self.largest_acked = self.largest_acked.max(acked_packet);

        // Remove acked packets and detect losses
        self.in_flight.retain(|packet| {
            if packet.packet_number <= acked_packet {
                acked_packets.push(packet.clone());
                false // Remove from in_flight
            } else {
                // Check for loss: either too many higher packets acked or timeout
                let higher_acked_count = self.largest_acked.saturating_sub(packet.packet_number);
                let timed_out =
                    current_time - packet.sent_time > self.loss_timeout.as_micros() as u64;

                if higher_acked_count >= self.loss_threshold || timed_out {
                    lost_packets.push(packet.clone());
                    false // Remove from in_flight
                } else {
                    true // Keep in in_flight
                }
            }
        });

        (acked_packets, lost_packets)
    }
}

/// Get current timestamp in microseconds
pub fn current_timestamp_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Calculate RTT from timestamp echo
pub fn calculate_rtt(ack_timestamp_echo: u64) -> Duration {
    let current = current_timestamp_micros();
    if current > ack_timestamp_echo {
        Duration::from_micros(current - ack_timestamp_echo)
    } else {
        Duration::from_micros(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_encode_decode() {
        let header = StpHeader::new(123, 456, 789);
        let mut buf = BytesMut::new();
        header.encode(&mut buf);

        let decoded = StpHeader::decode(buf.freeze()).unwrap();
        assert_eq!(header.packet_number, decoded.packet_number);
        assert_eq!(header.latest_ack, decoded.latest_ack);
        assert_eq!(header.ack_timestamp_echo, decoded.ack_timestamp_echo);
    }

    #[test]
    fn test_packet_encode_decode() {
        let payload = Bytes::from_static(b"hello world");
        let packet = StpPacket::new(1, 0, 0, payload.clone());

        let encoded = packet.encode();
        let decoded = StpPacket::decode(encoded).unwrap();

        assert_eq!(packet.header.packet_number, decoded.header.packet_number);
        assert_eq!(packet.payload, decoded.payload);
    }
}
