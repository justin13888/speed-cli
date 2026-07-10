//! Suite reports must render as a single self-contained HTML document:
//! one shell, every phase embedded, hostile strings escaped.

mod common;

use common::{make_phase_params, make_sample_report, make_sample_suite};
use speed_cli::TestType;
use speed_cli::renderer::ToHtml;
use speed_cli::report::SuiteReport;

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn suite_html_is_single_document_with_all_phases() {
    let suite = make_sample_suite();
    let html = suite.to_html();

    // Embedded phases must not emit nested documents.
    assert_eq!(count_occurrences(&html, "<!DOCTYPE"), 1);
    assert_eq!(count_occurrences(&html, "</html>"), 1);
    assert_eq!(count_occurrences(&html, "<style>"), 1);

    assert!(html.contains("tcp/bidirectional"));
    assert!(html.contains("http1/latency"));
    assert!(html.contains(r#"id="phase-0""#));
    assert!(html.contains(r#"id="phase-1""#));
    assert!(html.contains(r##"href="#phase-0""##));

    // The throughput phase renders the shared per-report sections.
    assert!(html.contains("Download Results"));
    // The data-less latency phase renders a dash headline, not a panic.
    assert!(html.contains("&mdash;"));
}

#[test]
fn suite_html_lists_skipped_phases_with_reasons() {
    let suite = make_sample_suite();
    let html = suite.to_html();

    assert!(html.contains("Skipped Phases"));
    assert!(html.contains("http3/bidirectional"));
    assert!(html.contains("server does not advertise the http3 test listener"));
}

#[test]
fn empty_suite_renders_valid_html() {
    let mut suite = SuiteReport::new("192.0.2.1".to_string());
    suite.environment = None;
    suite.finalize();
    let html = suite.to_html();

    assert_eq!(count_occurrences(&html, "<!DOCTYPE"), 1);
    assert_eq!(count_occurrences(&html, "</html>"), 1);
    assert!(html.contains("No phases completed"));
    assert!(!html.contains("<h2 class=\"section-title\">Environment</h2>"));
    assert!(!html.contains("Skipped Phases"));
}

#[test]
fn suite_html_escapes_labels_and_reasons() {
    let mut suite = SuiteReport::new("evil<host>".to_string());
    suite.record(
        "http/<script>alert(1)</script>",
        make_phase_params(TestType::Bidirectional, Some(1024)),
        make_sample_report(),
    );
    suite.skip("bad/phase", r#"error: <img src="x">"#);
    suite.finalize();
    let html = suite.to_html();

    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("evil<host>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("evil&lt;host&gt;"));
}
