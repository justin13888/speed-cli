//! Shared fixtures for report/suite serialization and rendering tests.
//! Each integration-test crate compiles this module independently, so
//! not every helper is used everywhere.
#![allow(dead_code)]

use std::time::Duration;

use chrono::Utc;
use speed_cli::TestType;
use speed_cli::performance::http::HttpVersion;
use speed_cli::report::{
    HttpTestConfig, NetworkTestResult, PhaseParams, Sample, StreamSamples, SuiteReport, TestConfig,
    TestReport, TestResult, ThroughputResult,
};

pub fn make_sample_report() -> TestReport {
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

/// A report whose result carries no throughput or latency data, the
/// shape a latency-only phase produces when nothing was measured.
pub fn make_empty_result_report() -> TestReport {
    let net = NetworkTestResult::new_http();
    let config = HttpTestConfig::new(
        "127.0.0.1".to_string(),
        Some(8080),
        1,
        1,
        TestType::LatencyOnly,
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

pub fn make_phase_params(test_type: TestType, payload_size: Option<usize>) -> PhaseParams {
    PhaseParams {
        payload_size,
        io_unit: 64 * 1024,
        connections: 4,
        duration: Duration::from_secs(8),
        test_type,
        deviations: Vec::new(),
    }
}

/// A two-phase suite (one throughput phase, one data-less latency
/// phase) with one skipped phase.
pub fn make_sample_suite() -> SuiteReport {
    let mut suite = SuiteReport::new("127.0.0.1".to_string());
    suite.record(
        "tcp/bidirectional",
        make_phase_params(TestType::Bidirectional, Some(1024)),
        make_sample_report(),
    );
    suite.record(
        "http1/latency",
        make_phase_params(TestType::LatencyOnly, None),
        make_empty_result_report(),
    );
    suite.skip(
        "http3/bidirectional",
        "server does not advertise the http3 test listener",
    );
    suite.finalize();
    suite
}
