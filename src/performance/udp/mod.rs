//! UDP blaster: a fixed-rate, no-retransmit UDP test protocol with
//! receiver-side loss / OOO / jitter accounting. See `protocol` for the
//! wire format. The previous STP / BBR implementation has been retired
//! - see the Phase 3 entry in the project plan for context.

pub mod client;
pub mod protocol;
pub mod server;
