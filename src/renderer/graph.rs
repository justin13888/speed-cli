//! Self-contained inline-SVG latency charts for the HTML report.
//!
//! [`latency_svg`] plots RTT against the test timeline with no JavaScript and
//! no external assets — just an `<svg>` the browser renders directly. It draws
//! the adaptive spike threshold as a reference line, marks dropped probes, and
//! optionally overlays a second series so an idle baseline can be compared
//! against latency measured under load (the bufferbloat view).
//!
//! Raw string literals here use `r##"..."##` because SVG attributes such as
//! `fill="#6c757d"` contain the `"#` sequence that would close an `r#"..."#`.

use crate::report::LatencyResult;

const W: f64 = 900.0;
const H: f64 = 260.0;
const ML: f64 = 54.0; // left margin (y labels)
const MR: f64 = 16.0;
const MT: f64 = 16.0;
const MB: f64 = 28.0; // bottom margin (x labels)

const PRIMARY_COLOR: &str = "#007acc";
const OVERLAY_COLOR: &str = "#28a745";
const THRESHOLD_COLOR: &str = "#dc3545";

/// Render a latency time-series (with an optional comparison overlay) as an
/// inline SVG `<div>`. Returns an empty string when there is no positive RTT
/// to plot, so the caller can skip the chart entirely.
///
/// `primary` is the headline series and supplies the spike threshold; when
/// present, `overlay` is drawn as a dashed reference line on the same axes.
pub fn latency_svg(primary: &LatencyResult, overlay: Option<&LatencyResult>) -> String {
    let plot_w = W - ML - MR;
    let plot_h = H - MT - MB;

    let series: Vec<&LatencyResult> = std::iter::once(primary).chain(overlay).collect();
    let t_max = series
        .iter()
        .flat_map(|r| r.measurements.iter())
        .map(|m| m.t_start_us)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let v_max = series
        .iter()
        .filter_map(|r| r.max_rtt())
        .fold(0.0_f64, f64::max);
    if v_max <= 0.0 {
        return String::new();
    }
    // A little vertical headroom so the worst spike isn't flush with the top.
    let v_top = v_max * 1.1;

    let x = |t_us: u64| ML + (t_us as f64 / t_max) * plot_w;
    let y = |v_ms: f64| MT + plot_h - (v_ms / v_top) * plot_h;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg viewBox="0 0 {W} {H}" width="100%" preserveAspectRatio="xMidYMid meet" style="max-width:{W}px; background:#fff; border:1px solid #e9ecef; border-radius:6px;">"##
    ));

    // Plot frame.
    svg.push_str(&format!(
        r##"<rect x="{ML}" y="{MT}" width="{plot_w}" height="{plot_h}" fill="#fbfcfd" stroke="#e9ecef"/>"##
    ));

    // Y labels at top (v_top) and bottom (0).
    let y0 = y(0.0);
    let ytop = y(v_top);
    svg.push_str(&format!(
        r##"<text x="{lx}" y="{ty}" font-size="11" fill="#6c757d" text-anchor="end">{vtop:.1} ms</text>"##,
        lx = ML - 6.0,
        ty = ytop + 10.0,
        vtop = v_top,
    ));
    svg.push_str(&format!(
        r##"<text x="{lx}" y="{by}" font-size="11" fill="#6c757d" text-anchor="end">0 ms</text>"##,
        lx = ML - 6.0,
        by = y0,
    ));

    // Spike threshold reference line (from the primary series).
    if let Some(sr) = primary.spike_report()
        && sr.threshold_ms <= v_top
    {
        let ty = y(sr.threshold_ms);
        svg.push_str(&format!(
            r##"<line x1="{ML}" y1="{ty}" x2="{x2}" y2="{ty}" stroke="{THRESHOLD_COLOR}" stroke-width="1" stroke-dasharray="5,4" opacity="0.8"/>"##,
            x2 = ML + plot_w,
        ));
        svg.push_str(&format!(
            r##"<text x="{tx}" y="{tyl}" font-size="10" fill="{THRESHOLD_COLOR}">spike &gt; {thr:.0} ms</text>"##,
            tx = ML + 4.0,
            tyl = ty - 3.0,
            thr = sr.threshold_ms,
        ));
    }

    // Overlay first (drawn underneath the primary), dashed.
    if let Some(ov) = overlay {
        svg.push_str(&polyline(ov, &x, &y, OVERLAY_COLOR, true));
    }
    svg.push_str(&polyline(primary, &x, &y, PRIMARY_COLOR, false));

    // Dropped-probe ticks along the baseline (primary series).
    for m in &primary.measurements {
        if m.rtt_us.is_none() {
            let px = x(m.t_start_us);
            svg.push_str(&format!(
                r##"<line x1="{px}" y1="{a}" x2="{px}" y2="{b}" stroke="{THRESHOLD_COLOR}" stroke-width="1" opacity="0.6"/>"##,
                a = y0,
                b = y0 - 6.0,
            ));
        }
    }

    // X-axis end labels.
    svg.push_str(&format!(
        r##"<text x="{ML}" y="{ly}" font-size="11" fill="#6c757d">0s</text>"##,
        ly = H - 8.0,
    ));
    svg.push_str(&format!(
        r##"<text x="{ex}" y="{ly}" font-size="11" fill="#6c757d" text-anchor="end">{secs:.1}s</text>"##,
        ex = ML + plot_w,
        ly = H - 8.0,
        secs = t_max / 1_000_000.0,
    ));

    svg.push_str("</svg>");

    let legend = if overlay.is_some() {
        format!(
            r##"<div style="font-size:12px; color:#6c757d; margin-top:6px;">
                <span style="color:{PRIMARY_COLOR};">●</span> under load
                &nbsp;&nbsp;<span style="color:{OVERLAY_COLOR};">●</span> idle baseline
                &nbsp;&nbsp;<span style="color:{THRESHOLD_COLOR};">┄</span> spike threshold / dropped
            </div>"##
        )
    } else {
        format!(
            r##"<div style="font-size:12px; color:#6c757d; margin-top:6px;">
                <span style="color:{PRIMARY_COLOR};">●</span> RTT
                &nbsp;&nbsp;<span style="color:{THRESHOLD_COLOR};">┄</span> spike threshold / dropped
            </div>"##
        )
    };

    format!(r##"<div style="margin:12px 0;">{svg}{legend}</div>"##)
}

/// Build a `<polyline>` through the successful probes of `result`.
fn polyline(
    result: &LatencyResult,
    x: &impl Fn(u64) -> f64,
    y: &impl Fn(f64) -> f64,
    color: &str,
    dashed: bool,
) -> String {
    let points: String = result
        .measurements
        .iter()
        .filter_map(|m| {
            m.rtt_ms()
                .map(|rtt| format!("{:.1},{:.1}", x(m.t_start_us), y(rtt)))
        })
        .collect::<Vec<_>>()
        .join(" ");
    if points.is_empty() {
        return String::new();
    }
    let dash = if dashed {
        r#" stroke-dasharray="6,4""#
    } else {
        ""
    };
    format!(
        r##"<polyline points="{points}" fill="none" stroke="{color}" stroke-width="1.5"{dash}/>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::LatencyMeasurement;
    use chrono::Utc;

    fn result(rtts_us: &[Option<u64>]) -> LatencyResult {
        LatencyResult {
            measurements: rtts_us
                .iter()
                .enumerate()
                .map(|(i, rtt)| LatencyMeasurement {
                    t_start_us: i as u64 * 10_000,
                    rtt_us: *rtt,
                })
                .collect(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn empty_when_no_positive_rtt() {
        assert!(latency_svg(&result(&[None, None]), None).is_empty());
        assert!(latency_svg(&result(&[]), None).is_empty());
    }

    #[test]
    fn renders_svg_with_polyline() {
        let svg = latency_svg(&result(&[Some(1_000), Some(2_000), Some(40_000)]), None);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("ms"));
    }

    #[test]
    fn overlay_adds_legend_and_second_line() {
        let loaded = result(&[Some(1_000), Some(60_000), Some(2_000)]);
        let idle = result(&[Some(1_000), Some(1_100), Some(1_050)]);
        let svg = latency_svg(&loaded, Some(&idle));
        assert!(svg.contains("idle baseline"));
        // Two polylines: primary + dashed overlay.
        assert_eq!(svg.matches("<polyline").count(), 2);
    }
}
