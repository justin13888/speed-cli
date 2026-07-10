//! Schema-version sanity. With CBOR-only export and no backwards
//! compatibility, the current `REPORT_SCHEMA_VERSION` is 9 and reports
//! at any other version are rejected at import time. This test is a
//! tripwire: bump it deliberately whenever the schema changes.

use speed_cli::report::REPORT_SCHEMA_VERSION;

#[test]
fn current_schema_version_is_nine() {
    assert_eq!(REPORT_SCHEMA_VERSION, 9);
}

/// The `congestion` field was added to `QuicTestConfig` *without* a
/// schema bump because it back-fills losslessly: every report written
/// before the knob existed ran cubic, which is exactly the serde
/// default. This pins that contract — a pre-field CBOR document must
/// decode with `congestion == Cubic`.
#[test]
fn quic_config_cbor_backfills_congestion() {
    use speed_cli::report::QuicTestConfig;
    use speed_cli::{CongestionAlgorithm, TestType};

    let config = QuicTestConfig::new(
        "127.0.0.1".to_string(),
        Some(4433),
        1,
        1,
        TestType::Bidirectional,
        vec![1024usize],
    );

    // Simulate a pre-field document: serialize to a CBOR map and strip
    // the `congestion` key, then decode into the current struct.
    let mut bytes = Vec::new();
    ciborium::into_writer(&config, &mut bytes).unwrap();
    let mut value: ciborium::Value = ciborium::from_reader(&bytes[..]).unwrap();
    let map = value.as_map_mut().expect("config encodes as a CBOR map");
    map.retain(|(k, _)| k.as_text() != Some("congestion"));
    let mut stripped = Vec::new();
    ciborium::into_writer(&value, &mut stripped).unwrap();

    let decoded: QuicTestConfig = ciborium::from_reader(&stripped[..]).unwrap();
    assert_eq!(decoded.congestion, CongestionAlgorithm::Cubic);
}
