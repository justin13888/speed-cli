//! Verify that a `TestReport` survives JSON and CBOR round-trips
//! (export → file → import) without losing measurement data.

use std::time::Duration;

use chrono::Utc;
use eyre::Result;
use indexmap::IndexSet;
use speed_cli::TestType;
use speed_cli::performance::http::HttpVersion;
use speed_cli::report::{
    HttpTestConfig, NetworkTestResult, TestConfig, TestReport, TestResult, ThroughputMeasurement,
    ThroughputResult,
};
use speed_cli::utils::export::{export_report_cbor, export_report_json};
use speed_cli::utils::import::{import_report_cbor, import_report_json};

fn make_sample_report() -> TestReport {
    let mut net = NetworkTestResult::new_http();
    let mut measurements = Vec::new();
    for _ in 0..5 {
        measurements.push(ThroughputMeasurement::new(1024, Duration::from_millis(10)));
    }
    let throughput = ThroughputResult {
        measurements,
        streams: Vec::new(),
        total_duration: Duration::from_secs(1),
        timestamp: Utc::now(),
        udp_stats: None,
    };
    net.download.insert(1024, throughput.clone());
    net.upload.insert(1024, throughput);

    let config = HttpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(8080),
        1,
        1,
        TestType::Bidirectional,
        Vec::<usize>::new(),
        Some(1024),
        HttpVersion::HTTP1,
    );

    TestReport::new(
        Utc::now(),
        TestConfig::Http(config),
        TestResult::Network(net),
        Utc::now(),
    )
}

fn assert_reports_equivalent(original: &TestReport, decoded: &TestReport) {
    assert_eq!(original.version, decoded.version);

    let (orig_net, dec_net) = match (&original.result, &decoded.result) {
        (TestResult::Network(a), TestResult::Network(b)) => (a, b),
        _ => panic!("expected NetworkTestResult on both sides"),
    };

    assert_eq!(orig_net.download.len(), dec_net.download.len());
    assert_eq!(orig_net.upload.len(), dec_net.upload.len());

    let orig_keys: IndexSet<_> = orig_net.download.keys().copied().collect();
    let dec_keys: IndexSet<_> = dec_net.download.keys().copied().collect();
    assert_eq!(orig_keys, dec_keys);

    for (size, orig) in &orig_net.download {
        let dec = dec_net.download.get(size).expect("matching download key");
        assert_eq!(orig.measurements.len(), dec.measurements.len());
        assert_eq!(orig.total_duration, dec.total_duration);
        assert_eq!(orig.bytes_transferred(), dec.bytes_transferred());
    }
}

#[tokio::test]
async fn json_roundtrip_preserves_report() -> Result<()> {
    let original = make_sample_report();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("report.json");

    export_report_json(&original, &path).await?;
    let decoded = import_report_json(&path).await?;

    assert_reports_equivalent(&original, &decoded);
    Ok(())
}

#[tokio::test]
async fn cbor_roundtrip_preserves_report() -> Result<()> {
    let original = make_sample_report();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("report.cbor");

    export_report_cbor(&original, &path).await?;
    let decoded = import_report_cbor(&path).await?;

    assert_reports_equivalent(&original, &decoded);
    Ok(())
}
