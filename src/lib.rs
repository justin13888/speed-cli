//! `speed-cli` — comprehensive multi-protocol network performance testing.
//!
//! Measures throughput, latency, jitter, and loss across TCP, UDP, raw QUIC,
//! HTTP/1.1, HTTP/2 (TLS and cleartext h2c), and HTTP/3. A server advertises
//! every enabled test listener through a single JSON control endpoint; clients
//! discover those ports and verify wire-protocol compatibility by handshaking
//! against it (see [`constants::PROTOCOL_VERSION`]).
//!
//! The CLI binary lives in `src/main.rs` and re-declares these modules with
//! its own `mod` directives; this `lib.rs` exists primarily so integration
//! tests under `tests/` (and any external embedders) can drive the
//! servers / clients / reporting types programmatically.

pub mod build_info;
pub mod cli;
pub mod constants;
pub mod control;
pub mod performance;
pub mod renderer;
pub mod report;
pub mod utils;

pub use utils::types::*;
