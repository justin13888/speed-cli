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

/// ALPN identifier for the raw-QUIC test protocol.
pub const QUIC_RAW_ALPN: &[u8] = b"speedcli-quic-raw";
