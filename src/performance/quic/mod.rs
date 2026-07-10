//! Raw QUIC stream throughput test — the QUIC analog of the raw-TCP
//! test.
//!
//! The wire shape mirrors the raw-TCP protocol: each QUIC bidirectional
//! stream carries one single-byte command (`'U'`/`'D'`/`'F'`/`'P'`/`'H'`)
//! followed by the corresponding data flow. Multiple parallel streams
//! ride a single QUIC connection — that multiplexing is the whole point
//! of measuring QUIC separately from TCP.
//!
//! A dedicated private ALPN (`speedcli-quic-raw`) keeps this listener
//! strictly distinct from the HTTP/3 listener: an HTTP/3 client and a
//! raw-QUIC client can never cross-connect because ALPN negotiation
//! fails.

pub mod client;
pub mod server;

use std::sync::Arc;

use crate::CongestionAlgorithm;

/// ALPN identifier for the raw-QUIC test protocol.
pub const QUIC_RAW_ALPN: &[u8] = b"speedcli-quic-raw";

/// Build the quinn transport config for the requested congestion
/// controller. CUBIC is quinn's default; it is set explicitly anyway so
/// the two arms stay symmetric and the choice is visible in one place.
/// Shared by the raw-QUIC client/server and the HTTP/3 server (the
/// HTTP/3 *client* is reqwest, which has its own BBR toggle).
pub fn quic_transport_config(congestion: CongestionAlgorithm) -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    match congestion {
        CongestionAlgorithm::Cubic => transport
            .congestion_controller_factory(Arc::new(quinn::congestion::CubicConfig::default())),
        CongestionAlgorithm::Bbr => transport
            .congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default())),
    };
    Arc::new(transport)
}
