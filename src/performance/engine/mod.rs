//! Shared measurement engine: the warmup-aware sampling primitives, the
//! generic stats collector, and progress-bar plumbing that every protocol
//! client builds on. Centralizing these keeps the per-protocol clients thin
//! and gives throughput/latency timing a single source of truth.

pub mod collector;
pub mod progress;
pub mod sampler;

pub use collector::{LatencyStatsCollector, ThroughputStatsCollector};
pub use progress::{ProgressBarType, create_progress_bar};
pub use sampler::{measurement_duration_us, offset_us};
