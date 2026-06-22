//! Warmup-aware sampling primitives shared by every protocol client.

use std::time::{Duration, Instant};

/// Effective measurement duration in microseconds: total elapsed minus the
/// warmup window, clamped so we never report a zero or negative duration that
/// would blow up throughput calculations.
///
/// NOTE: the 1ms floor masks the `warmup >= elapsed` case; CLI-level
/// validation (and a principled minimum window) is layered on top so this
/// clamp is only ever a last-resort guard.
pub fn measurement_duration_us(start: Instant, end: Instant, warmup: Duration) -> u64 {
    end.duration_since(start)
        .saturating_sub(warmup)
        .max(Duration::from_millis(1))
        .as_micros() as u64
}

/// Offset of `now` from the test start in microseconds — the sample's
/// position on the test's monotonic time axis (where 0 is the test start).
#[inline]
pub fn offset_us(start: Instant, now: Instant) -> u64 {
    now.duration_since(start).as_micros() as u64
}
