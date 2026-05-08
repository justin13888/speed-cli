//! A pre-2.x report (no `schema_version`, no `streams`, no `accounting`)
//! must still deserialize cleanly through the `#[serde(default)]`
//! fallbacks - that's the whole point of bumping the schema_version
//! field instead of breaking compatibility.

use eyre::Result;
use speed_cli::report::{NetworkProtocol, TestReport, TestResult, ThroughputAccounting};

#[test]
fn legacy_report_loads_with_defaults() -> Result<()> {
    // Hand-rolled JSON that mirrors the v0 (pre-schema-versioning)
    // shape: no `schema_version`, no `streams`, no `accounting`.
    let json = r#"{
        "start_time": "2025-01-01T00:00:00Z",
        "config": {
            "tcp": {
                "server": "127.0.0.1",
                "port": 5201,
                "duration": { "secs": 10, "nanos": 0 },
                "parallel_connections": 1,
                "test_type": "bidirectional",
                "payload_sizes": [1024]
            }
        },
        "result": {
            "Network": {
                "latency": null,
                "download": {
                    "1024": {
                        "measurements": [
                            { "type": "success", "bytes": 1024, "duration": { "secs": 0, "nanos": 1000000 } }
                        ],
                        "total_duration": { "secs": 1, "nanos": 0 },
                        "timestamp": "2025-01-01T00:00:01Z"
                    }
                },
                "upload": {},
                "protocol": "Tcp"
            }
        },
        "timestamp": "2025-01-01T00:00:01Z",
        "version": "0.1.0"
    }"#;

    let report: TestReport = serde_json::from_str(json)?;
    assert_eq!(report.schema_version, 0, "missing schema_version → default 0");

    let net = match &report.result {
        TestResult::Network(n) => n,
        _ => panic!("expected Network result"),
    };
    assert!(matches!(net.protocol, NetworkProtocol::Tcp));
    assert!(
        matches!(net.accounting, ThroughputAccounting::Goodput),
        "missing accounting → defaults to Goodput"
    );

    let download = net.download.values().next().expect("download present");
    assert!(
        download.streams.is_empty(),
        "missing streams → defaults to empty Vec"
    );
    assert_eq!(download.bytes_transferred(), 1024);
    Ok(())
}
