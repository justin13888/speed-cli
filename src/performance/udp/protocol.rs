//! Wire format for the UDP blaster protocol.
//!
//! Replaces the previous STP / BBR implementation. The blaster is a
//! simple iperf3-u-style protocol: a fixed-rate sender, no
//! retransmissions, server-side counting of received / lost / OOO /
//! duplicate packets and RFC 3550 interarrival jitter. Use it to
//! characterize raw UDP loss/jitter/throughput between two endpoints,
//! not to do congestion-controlled streaming.

use bytes::{Buf, BufMut, Bytes, BytesMut};

/// 32-bit magic to disambiguate from random UDP traffic. ASCII "BLST".
pub const MAGIC: u32 = 0x424C5354;

/// Fixed minimum header (magic + kind = 5 bytes). Bodies follow per kind.
pub const MIN_HEADER_SIZE: usize = 5;

/// Direction of a blaster session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Server sends, client receives.
    Download,
    /// Client sends, server receives.
    Upload,
    /// Single PING/PONG round-trips.
    Latency,
}

impl Mode {
    fn to_u8(self) -> u8 {
        match self {
            Mode::Download => 0,
            Mode::Upload => 1,
            Mode::Latency => 2,
        }
    }
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Mode::Download),
            1 => Some(Mode::Upload),
            2 => Some(Mode::Latency),
            _ => None,
        }
    }
}

const KIND_START: u8 = 1;
const KIND_DATA: u8 = 2;
const KIND_FIN: u8 = 3;
const KIND_REPORT: u8 = 4;
const KIND_PING: u8 = 5;
const KIND_PONG: u8 = 6;
const KIND_HELLO: u8 = 7;
const KIND_HELLO_ACK: u8 = 8;

const HELLO_BLOB_MAX: u16 = 4096;

/// One blaster control or data packet.
#[derive(Debug, Clone)]
pub enum BlasterPacket {
    /// Client → Server: begin a session.
    Start {
        mode: Mode,
        /// Target send rate in bits per second. 0 means "saturate".
        target_rate_bps: u64,
        /// Payload size for DATA packets.
        payload_size: u32,
        /// Test duration in milliseconds (informational).
        duration_ms: u64,
    },
    /// Bulk data packet. Sequence numbers start at 1.
    Data {
        seq: u64,
        /// Sender's `now_us()` at send time, used for jitter calc on
        /// the receiver.
        send_ts_us: u64,
    },
    /// Client → Server: end of test, request a REPORT.
    Fin,
    /// Server → Client: end-of-test summary.
    Report {
        received: u64,
        bytes_received: u64,
        /// `lost = max_seq - (received - duplicates)`. Saturates at 0
        /// when reordering pushes a sequence number past the highest
        /// seen.
        lost: u64,
        out_of_order: u64,
        /// RFC 3550 interarrival jitter in microseconds.
        jitter_us: u64,
        /// Packets whose sequence number had been seen before. Older
        /// peers that don't include this field decode as `0`.
        duplicates: u64,
    },
    /// Latency probe.
    Ping { send_ts_us: u64 },
    /// Latency response. `send_ts_us` echoes the corresponding Ping.
    Pong { send_ts_us: u64 },
    /// Client → Server: identity handshake. `identity_cbor` is a
    /// CBOR-encoded `PeerIdentity` (kept opaque at the protocol layer
    /// so this module doesn't depend on the report types).
    Hello {
        identity_cbor: Vec<u8>,
        t_send_us: u64,
    },
    /// Server → Client: response to `Hello`, plus server's view of the
    /// observed client address (CBOR-encoded `SocketAddr`) and the
    /// server's epoch in microseconds (so periodic-stats series — when
    /// they ship — line up with client-side time offsets).
    HelloAck {
        identity_cbor: Vec<u8>,
        observed_client_addr_cbor: Vec<u8>,
        server_epoch_us: u64,
    },
}

impl BlasterPacket {
    /// Encode this packet plus an optional trailing payload (for DATA).
    pub fn encode_to_vec(&self, payload: Option<&[u8]>) -> Bytes {
        let mut out = BytesMut::with_capacity(64 + payload.map_or(0, |p| p.len()));
        out.put_u32(MAGIC);
        match self {
            BlasterPacket::Start {
                mode,
                target_rate_bps,
                payload_size,
                duration_ms,
            } => {
                out.put_u8(KIND_START);
                out.put_u8(mode.to_u8());
                out.put_u16(0); // padding
                out.put_u64(*target_rate_bps);
                out.put_u32(*payload_size);
                out.put_u64(*duration_ms);
            }
            BlasterPacket::Data { seq, send_ts_us } => {
                out.put_u8(KIND_DATA);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*seq);
                out.put_u64(*send_ts_us);
                if let Some(p) = payload {
                    out.put_slice(p);
                }
            }
            BlasterPacket::Fin => {
                out.put_u8(KIND_FIN);
            }
            BlasterPacket::Report {
                received,
                bytes_received,
                lost,
                out_of_order,
                jitter_us,
                duplicates,
            } => {
                out.put_u8(KIND_REPORT);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*received);
                out.put_u64(*bytes_received);
                out.put_u64(*lost);
                out.put_u64(*out_of_order);
                out.put_u64(*jitter_us);
                out.put_u64(*duplicates);
            }
            BlasterPacket::Ping { send_ts_us } => {
                out.put_u8(KIND_PING);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*send_ts_us);
            }
            BlasterPacket::Pong { send_ts_us } => {
                out.put_u8(KIND_PONG);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*send_ts_us);
            }
            BlasterPacket::Hello {
                identity_cbor,
                t_send_us,
            } => {
                out.put_u8(KIND_HELLO);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*t_send_us);
                let len = identity_cbor.len().min(HELLO_BLOB_MAX as usize);
                out.put_u16(len as u16);
                out.put_slice(&identity_cbor[..len]);
            }
            BlasterPacket::HelloAck {
                identity_cbor,
                observed_client_addr_cbor,
                server_epoch_us,
            } => {
                out.put_u8(KIND_HELLO_ACK);
                out.put_u8(0);
                out.put_u16(0);
                out.put_u64(*server_epoch_us);
                let id_len = identity_cbor.len().min(HELLO_BLOB_MAX as usize);
                out.put_u16(id_len as u16);
                out.put_slice(&identity_cbor[..id_len]);
                let addr_len = observed_client_addr_cbor.len().min(HELLO_BLOB_MAX as usize);
                out.put_u16(addr_len as u16);
                out.put_slice(&observed_client_addr_cbor[..addr_len]);
            }
        }
        out.freeze()
    }

    /// Decode a packet. For DATA, the trailing payload is everything
    /// after the fixed 24-byte header; we return its length here, the
    /// raw payload is read by the caller from the original buffer.
    ///
    /// Zero-copy: we parse straight off the borrowed slice via
    /// `impl Buf for &[u8]`, which advances the slice reference in place
    /// instead of allocating. The hot DATA / PING / PONG paths never copy
    /// the datagram; only the rare `Hello`/`HelloAck` control packets copy
    /// their small CBOR blobs out into owned `Vec`s.
    pub fn decode(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < MIN_HEADER_SIZE {
            return None;
        }
        let mut buf: &[u8] = data;
        let magic = buf.get_u32();
        if magic != MAGIC {
            return None;
        }
        let kind = buf.get_u8();
        match kind {
            KIND_START => {
                // mode(1) + pad(2) + target_rate_bps(8) + payload_size(4) + duration_ms(8) = 23
                if buf.remaining() < 1 + 2 + 8 + 4 + 8 {
                    return None;
                }
                let mode = Mode::from_u8(buf.get_u8())?;
                let _ = buf.get_u16();
                let target_rate_bps = buf.get_u64();
                let payload_size = buf.get_u32();
                let duration_ms = buf.get_u64();
                Some((
                    BlasterPacket::Start {
                        mode,
                        target_rate_bps,
                        payload_size,
                        duration_ms,
                    },
                    0,
                ))
            }
            KIND_DATA => {
                // pad(3) + seq(8) + send_ts_us(8) = 19, then trailing payload
                if buf.remaining() < 1 + 2 + 8 + 8 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let seq = buf.get_u64();
                let send_ts_us = buf.get_u64();
                let payload_len = buf.remaining();
                Some((BlasterPacket::Data { seq, send_ts_us }, payload_len))
            }
            KIND_FIN => Some((BlasterPacket::Fin, 0)),
            KIND_REPORT => {
                if buf.remaining() < 1 + 2 + 8 * 5 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let received = buf.get_u64();
                let bytes_received = buf.get_u64();
                let lost = buf.get_u64();
                let out_of_order = buf.get_u64();
                let jitter_us = buf.get_u64();
                // duplicates was added later; treat absence as 0 so we
                // can decode reports from older peers without crashing.
                let duplicates = if buf.remaining() >= 8 {
                    buf.get_u64()
                } else {
                    0
                };
                Some((
                    BlasterPacket::Report {
                        received,
                        bytes_received,
                        lost,
                        out_of_order,
                        jitter_us,
                        duplicates,
                    },
                    0,
                ))
            }
            KIND_PING => {
                if buf.remaining() < 1 + 2 + 8 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let send_ts_us = buf.get_u64();
                Some((BlasterPacket::Ping { send_ts_us }, 0))
            }
            KIND_PONG => {
                if buf.remaining() < 1 + 2 + 8 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let send_ts_us = buf.get_u64();
                Some((BlasterPacket::Pong { send_ts_us }, 0))
            }
            KIND_HELLO => {
                if buf.remaining() < 1 + 2 + 8 + 2 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let t_send_us = buf.get_u64();
                let id_len = buf.get_u16() as usize;
                if buf.remaining() < id_len {
                    return None;
                }
                let mut identity_cbor = vec![0u8; id_len];
                buf.copy_to_slice(&mut identity_cbor);
                Some((
                    BlasterPacket::Hello {
                        identity_cbor,
                        t_send_us,
                    },
                    0,
                ))
            }
            KIND_HELLO_ACK => {
                if buf.remaining() < 1 + 2 + 8 + 2 {
                    return None;
                }
                let _ = buf.get_u8();
                let _ = buf.get_u16();
                let server_epoch_us = buf.get_u64();
                let id_len = buf.get_u16() as usize;
                if buf.remaining() < id_len + 2 {
                    return None;
                }
                let mut identity_cbor = vec![0u8; id_len];
                buf.copy_to_slice(&mut identity_cbor);
                let addr_len = buf.get_u16() as usize;
                if buf.remaining() < addr_len {
                    return None;
                }
                let mut observed_client_addr_cbor = vec![0u8; addr_len];
                buf.copy_to_slice(&mut observed_client_addr_cbor);
                Some((
                    BlasterPacket::HelloAck {
                        identity_cbor,
                        observed_client_addr_cbor,
                        server_epoch_us,
                    },
                    0,
                ))
            }
            _ => None,
        }
    }
}

/// Reusable encoder for DATA packets. A DATA packet is a fixed 24-byte header
/// (magic, kind, padding, seq, send_ts_us) followed by a payload that stays
/// constant for the life of a blaster session. Rather than allocate a fresh
/// buffer and re-copy the payload on every packet (as [`BlasterPacket::encode_to_vec`]
/// does), this renders the whole packet once and then rewrites only the 16
/// mutable header bytes — `seq` and `send_ts_us` — per packet. The bytes it
/// produces are byte-for-byte identical to
/// `BlasterPacket::Data { seq, send_ts_us }.encode_to_vec(Some(payload))`.
pub struct DataPacketWriter {
    buf: Vec<u8>,
}

impl DataPacketWriter {
    /// magic(4) + kind(1) + pad(3) + seq(8) + send_ts_us(8).
    const HEADER_LEN: usize = 24;
    const SEQ_OFFSET: usize = 8;
    const TS_OFFSET: usize = 16;

    /// Build a writer for a session whose DATA packets all carry `payload`.
    pub fn new(payload: &[u8]) -> Self {
        let mut buf = vec![0u8; Self::HEADER_LEN + payload.len()];
        buf[0..4].copy_from_slice(&MAGIC.to_be_bytes());
        buf[4] = KIND_DATA;
        // buf[5..8] is padding, already zero; seq/ts are filled by `frame`.
        buf[Self::HEADER_LEN..].copy_from_slice(payload);
        Self { buf }
    }

    /// Rewrite `seq` and `send_ts_us` in place and return the full packet.
    pub fn frame(&mut self, seq: u64, send_ts_us: u64) -> &[u8] {
        self.buf[Self::SEQ_OFFSET..Self::SEQ_OFFSET + 8].copy_from_slice(&seq.to_be_bytes());
        self.buf[Self::TS_OFFSET..Self::TS_OFFSET + 8].copy_from_slice(&send_ts_us.to_be_bytes());
        &self.buf
    }
}

/// Tracks per-session loss / OOO / duplicates / jitter on the
/// receiving side. Loss is computed at report time so that reordering
/// across the highest-seq mark doesn't get counted as a permanent loss.
#[derive(Debug, Default)]
pub struct ReceiveStats {
    pub max_seq: u64,
    pub received: u64,
    pub bytes_received: u64,
    pub out_of_order: u64,
    pub duplicates: u64,
    /// Smoothed RFC 3550 interarrival jitter in microseconds.
    jitter: f64,
    /// Previous (recv_us - send_us) sample for jitter calc.
    prev_transit_us: Option<i128>,
    /// Sliding "we've seen this seq before" set for the most recent
    /// REORDER_WINDOW packets. Anything older we ignore for duplicate
    /// detection - dups across a multi-second window aren't useful
    /// signal.
    recent_seqs: std::collections::VecDeque<u64>,
    recent_seqs_set: std::collections::HashSet<u64>,
}

/// How far back we'll detect duplicates. Packets reordered by more
/// than this are not counted as duplicates (they'll just look like
/// out-of-order receives, which they functionally are).
const REORDER_WINDOW: usize = 4096;

impl ReceiveStats {
    pub fn record(&mut self, seq: u64, payload_bytes: u64, send_ts_us: u64, recv_ts_us: u64) {
        self.received += 1;
        self.bytes_received += payload_bytes;

        if self.recent_seqs_set.contains(&seq) {
            self.duplicates += 1;
        } else {
            self.recent_seqs.push_back(seq);
            self.recent_seqs_set.insert(seq);
            if self.recent_seqs.len() > REORDER_WINDOW
                && let Some(old) = self.recent_seqs.pop_front()
            {
                self.recent_seqs_set.remove(&old);
            }
        }

        if seq > self.max_seq {
            self.max_seq = seq;
        } else {
            self.out_of_order += 1;
        }

        let cur = recv_ts_us as i128 - send_ts_us as i128;
        if let Some(prev) = self.prev_transit_us {
            let d = (cur - prev).abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.prev_transit_us = Some(cur);
    }

    /// `lost = max_seq - (received - duplicates)`. Subtracting
    /// duplicates avoids counting a re-sent packet as if it filled a
    /// hole. Saturates at 0 to handle pathological reorder.
    pub fn lost(&self) -> u64 {
        let unique = self.received.saturating_sub(self.duplicates);
        self.max_seq.saturating_sub(unique)
    }

    pub fn jitter_us(&self) -> u64 {
        self.jitter.round().max(0.0) as u64
    }
}

/// Microseconds since the Unix epoch. We use system time rather than
/// `Instant` because the value is wire-encoded and only used for jitter
/// calculations where the receiver subtracts paired samples - clock
/// drift between hosts cancels out.
pub fn now_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_start() {
        let p = BlasterPacket::Start {
            mode: Mode::Upload,
            target_rate_bps: 1_000_000,
            payload_size: 1400,
            duration_ms: 5_000,
        };
        let bytes = p.encode_to_vec(None);
        let (decoded, _) = BlasterPacket::decode(&bytes).unwrap();
        match decoded {
            BlasterPacket::Start {
                mode,
                target_rate_bps,
                payload_size,
                duration_ms,
            } => {
                assert_eq!(mode, Mode::Upload);
                assert_eq!(target_rate_bps, 1_000_000);
                assert_eq!(payload_size, 1400);
                assert_eq!(duration_ms, 5_000);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn roundtrip_data_with_payload() {
        let p = BlasterPacket::Data {
            seq: 42,
            send_ts_us: 12345,
        };
        let payload = vec![0xAB; 64];
        let bytes = p.encode_to_vec(Some(&payload));
        let (decoded, payload_len) = BlasterPacket::decode(&bytes).unwrap();
        match decoded {
            BlasterPacket::Data { seq, send_ts_us } => {
                assert_eq!(seq, 42);
                assert_eq!(send_ts_us, 12345);
                assert_eq!(payload_len, 64);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bad = vec![0u8; 32];
        bad[0..4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(BlasterPacket::decode(&bad).is_none());
    }

    #[test]
    fn receive_stats_loss_and_jitter() {
        let mut s = ReceiveStats::default();
        // seqs 1, 2, 4 → received 3, max_seq 4, lost 1
        s.record(1, 100, 0, 100);
        s.record(2, 100, 100, 200);
        s.record(4, 100, 300, 400);
        assert_eq!(s.received, 3);
        assert_eq!(s.max_seq, 4);
        assert_eq!(s.lost(), 1);
        assert_eq!(s.duplicates, 0);
    }

    #[test]
    fn receive_stats_counts_duplicates_and_excludes_them_from_loss() {
        // Re-sent packets (e.g. middlebox replay) should be counted as
        // duplicates and *not* as filling a gap. seqs: 1,2,2,4 → 3 unique,
        // 1 dup, max_seq 4 → lost = 4 - 3 = 1.
        let mut s = ReceiveStats::default();
        s.record(1, 100, 0, 100);
        s.record(2, 100, 100, 200);
        s.record(2, 100, 100, 250); // duplicate
        s.record(4, 100, 300, 400);
        assert_eq!(s.received, 4);
        assert_eq!(s.duplicates, 1);
        assert_eq!(s.lost(), 1);
    }

    #[test]
    fn report_packet_roundtrip_includes_duplicates() {
        let p = BlasterPacket::Report {
            received: 100,
            bytes_received: 100_000,
            lost: 5,
            out_of_order: 2,
            jitter_us: 42,
            duplicates: 3,
        };
        let bytes = p.encode_to_vec(None);
        let (decoded, _) = BlasterPacket::decode(&bytes).unwrap();
        match decoded {
            BlasterPacket::Report {
                duplicates, lost, ..
            } => {
                assert_eq!(duplicates, 3);
                assert_eq!(lost, 5);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn receive_stats_jitter_responds_to_variation() {
        // Constant transit time → jitter stays zero.
        let mut steady = ReceiveStats::default();
        steady.record(1, 0, 0, 100);
        steady.record(2, 0, 100, 200);
        steady.record(3, 0, 200, 300);
        assert_eq!(steady.jitter_us(), 0);

        // Variable transit time → jitter > 0.
        let mut jittery = ReceiveStats::default();
        jittery.record(1, 0, 0, 100);
        jittery.record(2, 0, 100, 350); // transit 250 (was 100)
        jittery.record(3, 0, 200, 400); // transit 200 (was 250)
        assert!(jittery.jitter_us() > 0);
    }

    #[test]
    fn data_packet_writer_matches_encode_to_vec() {
        let payload = vec![0xCDu8; 200];
        let mut w = DataPacketWriter::new(&payload);
        // Reusing the same writer across packets must keep producing bytes
        // identical to a fresh encode_to_vec, and they must decode back.
        for (seq, ts) in [(1u64, 42u64), (123, 456_789), (u64::MAX, 0)] {
            let framed = w.frame(seq, ts).to_vec();
            let reference = BlasterPacket::Data {
                seq,
                send_ts_us: ts,
            }
            .encode_to_vec(Some(&payload));
            assert_eq!(framed, reference.as_ref(), "seq={seq} ts={ts}");
            let (decoded, plen) = BlasterPacket::decode(&framed).unwrap();
            assert!(
                matches!(decoded, BlasterPacket::Data { seq: s, send_ts_us: t } if s == seq && t == ts)
            );
            assert_eq!(plen, payload.len());
        }
    }

    #[test]
    fn data_packet_writer_empty_payload() {
        let mut w = DataPacketWriter::new(&[]);
        let framed = w.frame(7, 9).to_vec();
        let reference = BlasterPacket::Data {
            seq: 7,
            send_ts_us: 9,
        }
        .encode_to_vec(Some(&[]));
        assert_eq!(framed, reference.as_ref());
    }
}
