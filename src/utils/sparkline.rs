//! Dependency-free terminal time-vs-latency chart.
//!
//! [`latency_sparkline`] bins latency probes by their test-timeline offset into
//! `width` columns and draws a `height`-row bar chart from Unicode block
//! glyphs. Bars are scaled from `0` to the worst RTT so spikes tower over the
//! steady-state baseline — exactly the shape that reveals a WiFi card / AP
//! misbehaving under load. Columns whose only probes were dropped are marked
//! with `x` on the baseline so packet loss reads alongside the latency.

use crate::report::LatencyMeasurement;

/// Eighths of a row, low to high. `BLOCKS[7]` is a full block.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Left gutter width for the y-axis value labels (e.g. `" 12.4 ms"`).
const LABEL_WIDTH: usize = 8;

/// Render a latency time-series as a multi-row terminal chart.
///
/// Returns an empty string when there is nothing to plot (the caller should
/// skip emitting a chart in that case). Never panics: `width`/`height` are
/// clamped to sane minimums and all-dropped input renders a marker row.
pub fn latency_sparkline(
    measurements: &[LatencyMeasurement],
    width: usize,
    height: usize,
) -> String {
    let width = width.max(8);
    let height = height.max(1);
    if measurements.is_empty() {
        return String::new();
    }

    let t_min = measurements.iter().map(|m| m.t_start_us).min().unwrap_or(0);
    let t_max = measurements.iter().map(|m| m.t_start_us).max().unwrap_or(0);
    let span = (t_max.saturating_sub(t_min)).max(1) as f64;

    // Per-column tail RTT (max within the bin) and a dropped-probe flag.
    let mut col_val = vec![f64::NAN; width];
    let mut col_drop = vec![false; width];
    for m in measurements {
        let frac = (m.t_start_us.saturating_sub(t_min)) as f64 / span;
        let col = ((frac * (width as f64 - 1.0)).round() as usize).min(width - 1);
        match m.rtt_ms() {
            Some(rtt) => {
                col_val[col] = if col_val[col].is_nan() {
                    rtt
                } else {
                    col_val[col].max(rtt)
                };
            }
            None => col_drop[col] = true,
        }
    }

    let max_v = col_val
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .fold(f64::MIN, f64::max);

    // No successful probe in any column: emit a single marker row.
    if !max_v.is_finite() {
        let markers: String = col_drop
            .iter()
            .map(|d| if *d { 'x' } else { ' ' })
            .collect();
        return format!("{:>width$} ┤{markers}", "drops", width = LABEL_WIDTH);
    }

    // Quantise each column to eighths across the full row stack.
    let levels_total = (height * 8) as f64;
    let fill: Vec<usize> = col_val
        .iter()
        .map(|v| {
            if v.is_nan() || max_v <= 0.0 {
                0
            } else {
                // Any positive sample fills at least one eighth so it stays visible.
                ((v / max_v) * levels_total).round().max(1.0) as usize
            }
        })
        .collect();

    let mut out = String::new();
    for row in (0..height).rev() {
        let label = if row == height - 1 {
            format!("{max_v:>5.1} ms")
        } else if row == 0 {
            format!("{:>5.1} ms", 0.0)
        } else {
            " ".repeat(LABEL_WIDTH)
        };
        out.push_str(&label);
        out.push('┤');
        for (col, &f) in fill.iter().enumerate() {
            let sub = f.saturating_sub(row * 8).min(8);
            let ch = if sub > 0 {
                BLOCKS[sub - 1]
            } else if row == 0 && col_drop[col] && f == 0 {
                // Bottom row marks columns that only ever dropped.
                'x'
            } else {
                ' '
            };
            out.push(ch);
        }
        out.push('\n');
    }

    // X-axis rule and end labels.
    out.push_str(&" ".repeat(LABEL_WIDTH));
    out.push('└');
    out.push_str(&"─".repeat(width));
    out.push('\n');

    let start_s = t_min as f64 / 1_000_000.0;
    let end_s = t_max as f64 / 1_000_000.0;
    let start_lbl = format!("{start_s:.1}s");
    let end_lbl = format!("{end_s:.1}s");
    let pad = (width + 1).saturating_sub(start_lbl.len() + end_lbl.len());
    out.push_str(&" ".repeat(LABEL_WIDTH + 1));
    out.push_str(&start_lbl);
    out.push_str(&" ".repeat(pad));
    out.push_str(&end_lbl);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(rtts_us: &[Option<u64>]) -> Vec<LatencyMeasurement> {
        rtts_us
            .iter()
            .enumerate()
            .map(|(i, rtt)| LatencyMeasurement {
                t_start_us: i as u64 * 10_000,
                rtt_us: *rtt,
            })
            .collect()
    }

    #[test]
    fn empty_series_renders_nothing() {
        assert!(latency_sparkline(&[], 40, 4).is_empty());
    }

    #[test]
    fn normal_series_has_axis_and_blocks() {
        let m = series(&[Some(1_000), Some(2_000), Some(50_000), Some(1_500)]);
        let out = latency_sparkline(&m, 40, 4);
        assert!(out.contains('┤'), "should draw a y-axis");
        assert!(out.contains('└'), "should draw an x-axis");
        assert!(
            out.contains('█'),
            "the 50 ms spike should reach a full block"
        );
        assert!(out.contains("ms"), "should label the value scale");
    }

    #[test]
    fn all_dropped_renders_marker_row_without_panic() {
        let m = series(&[None, None, None]);
        let out = latency_sparkline(&m, 16, 4);
        assert!(out.contains('x'), "dropped-only series should be marked");
        assert!(!out.contains('█'));
    }

    #[test]
    fn width_and_height_are_clamped() {
        let m = series(&[Some(1_000), Some(2_000)]);
        // Degenerate dimensions must not panic.
        let out = latency_sparkline(&m, 0, 0);
        assert!(!out.is_empty());
    }
}
