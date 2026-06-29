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

/// Classify a *completed* sample as warmup.
///
/// `completed_offset_us` is the sample's completion time on the test's monotonic
/// axis (`t_start_us + duration_us`). A sample is warmup only when it *finishes*
/// inside the warmup window — not merely when it starts. A slow request that
/// begins during warmup but completes well into the measurement window (e.g. a
/// single 8 MB HTTP download that spans the whole test under full-duplex
/// contention) carried the bulk of its bytes post-warmup and must be counted;
/// tagging at the *start* would discard such a link-spanning request entirely
/// and report a false zero.
#[inline]
pub fn sample_is_warmup(completed_offset_us: u64, warmup: Duration) -> bool {
    completed_offset_us < warmup.as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanning_request_completing_after_warmup_is_measurement() {
        let warmup = Duration::from_secs(1);
        // The bug scenario: a single 8 MB download starts at t≈0 (inside warmup)
        // but takes 9.65 s under full-duplex contention. It completes long after
        // warmup, so it must count — tagging at start would zero the whole test.
        assert!(!sample_is_warmup(9_650_000, warmup));
        // A quick request that both starts and finishes inside warmup is warmup.
        assert!(sample_is_warmup(500_000, warmup));
    }

    #[test]
    fn warmup_boundary_is_half_open() {
        let warmup = Duration::from_secs(1);
        // Completing exactly at the warmup edge counts as measurement; a hair
        // before it is still warmup.
        assert!(!sample_is_warmup(1_000_000, warmup));
        assert!(sample_is_warmup(999_999, warmup));
    }

    #[test]
    fn zero_warmup_never_tags() {
        assert!(!sample_is_warmup(0, Duration::ZERO));
        assert!(!sample_is_warmup(1, Duration::ZERO));
    }
}
