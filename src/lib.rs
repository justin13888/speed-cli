//! Library surface for `speed-cli`.
//!
//! The CLI binary lives in `src/main.rs` and re-declares these modules with
//! its own `mod` directives; this `lib.rs` exists primarily so integration
//! tests under `tests/` (and any external embedders) can drive the
//! servers / clients / reporting types programmatically.

pub mod cli;
pub mod constants;
pub mod performance;
pub mod renderer;
pub mod report;
pub mod utils;

pub use utils::types::*;
