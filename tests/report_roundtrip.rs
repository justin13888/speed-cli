//! Verify that a `TestReport` survives a CBOR round-trip
//! (export → file → import) without losing per-sample data.

use chrono::Utc;
use eyre::Result;
use indexmap::IndexSet;
use speed_cli::TestType;
use speed_cli::performance::http::HttpVersion;
use speed_cli::report::{
    HttpTestConfig, NetworkTestResult, Sample, StreamSamples, TestConfig, TestReport, TestResult,
    ThroughputResult,
};
use speed_cli::utils::export::export_report_cbor;
use speed_cli::utils::import::import_report_cbor;

fn make_sample_report() -> TestReport {
    let mut net = NetworkTestResult::new_http();
    let samples: Vec<Sample> = (0..5)
        .map(|i| Sample::success(i * 10_000, 10_000, 1024, false))
        .collect();
    let stream_start = samples.first().map(|s| s.t_start_us).unwrap_or(0);
    let throughput = ThroughputResult {
        streams: vec![StreamSamples {
            stream_id: 0,
            start_offset_us: stream_start,
            samples,
        }],
        total_duration_us: 1_000_000,
        timestamp: Utc::now(),
        udp_stats: None,
        udp_series: Vec::new(),
        udp_series_window_us: 0,
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
    assert_eq!(original.schema_version, decoded.schema_version);

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
        assert_eq!(orig.streams.len(), dec.streams.len());
        for (a, b) in orig.streams.iter().zip(dec.streams.iter()) {
            assert_eq!(a.stream_id, b.stream_id);
            assert_eq!(a.start_offset_us, b.start_offset_us);
            assert_eq!(a.samples.len(), b.samples.len());
            for (sa, sb) in a.samples.iter().zip(b.samples.iter()) {
                assert_eq!(sa.t_start_us, sb.t_start_us);
                assert_eq!(sa.duration_us, sb.duration_us);
                assert_eq!(sa.bytes, sb.bytes);
                assert_eq!(sa.is_warmup, sb.is_warmup);
            }
        }
        assert_eq!(orig.total_duration_us, dec.total_duration_us);
        assert_eq!(orig.bytes_transferred(), dec.bytes_transferred());
    }
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

/// Time-monotonicity property within a single stream:
/// `samples[i].t_start_us + duration_us <= samples[i+1].t_start_us`.
#[tokio::test]
async fn samples_within_stream_are_time_monotonic() -> Result<()> {
    let report = make_sample_report();
    let TestResult::Network(net) = &report.result else {
        panic!("expected Network result");
    };
    for (_, t) in net.download.iter().chain(net.upload.iter()) {
        for stream in &t.streams {
            for pair in stream.samples.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                assert!(
                    a.t_start_us + a.duration_us <= b.t_start_us,
                    "intra-stream time monotonicity violated: a.t={} a.d={} b.t={}",
                    a.t_start_us,
                    a.duration_us,
                    b.t_start_us
                );
            }
        }
    }
    Ok(())
}
