//! Suite-report CBOR round-trips and the auto-detection contract of
//! `import_any_report_cbor`.

mod common;

use common::{make_sample_report, make_sample_suite};
use eyre::Result;
use speed_cli::report::{REPORT_SCHEMA_VERSION, SuiteReport, TestReport};
use speed_cli::utils::export::export_report_cbor;
use speed_cli::utils::import::{
    ImportError, LoadedReport, import_any_report_cbor, import_suite_report_cbor,
};

#[tokio::test]
async fn suite_cbor_roundtrip_preserves_report() -> Result<()> {
    let original = make_sample_suite();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("suite.cbor");

    export_report_cbor(&original, &path).await?;
    let decoded = import_suite_report_cbor(&path).await?;

    assert_eq!(decoded.schema_version, REPORT_SCHEMA_VERSION);
    assert_eq!(decoded.build, original.build);
    assert_eq!(decoded.server, original.server);
    assert_eq!(decoded.reports.len(), original.reports.len());
    for (orig, dec) in original.reports.iter().zip(decoded.reports.iter()) {
        assert_eq!(orig.label, dec.label);
        assert_eq!(orig.params.payload_size, dec.params.payload_size);
        assert_eq!(orig.params.io_unit, dec.params.io_unit);
        assert_eq!(orig.params.connections, dec.params.connections);
        assert_eq!(orig.params.duration, dec.params.duration);
    }
    assert_eq!(decoded.skipped.len(), original.skipped.len());
    assert_eq!(decoded.skipped[0].label, original.skipped[0].label);
    assert_eq!(decoded.skipped[0].reason, original.skipped[0].reason);
    Ok(())
}

#[tokio::test]
async fn import_any_detects_single_report() -> Result<()> {
    let report = make_sample_report();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("single.cbor");
    export_report_cbor(&report, &path).await?;

    match import_any_report_cbor(&path).await? {
        LoadedReport::Single(r) => assert_eq!(r.schema_version, REPORT_SCHEMA_VERSION),
        LoadedReport::Suite(_) => panic!("single-test CBOR detected as suite"),
    }
    Ok(())
}

#[tokio::test]
async fn import_any_detects_suite_report() -> Result<()> {
    let suite = make_sample_suite();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("suite.cbor");
    export_report_cbor(&suite, &path).await?;

    match import_any_report_cbor(&path).await? {
        LoadedReport::Suite(s) => assert_eq!(s.reports.len(), suite.reports.len()),
        LoadedReport::Single(_) => panic!("suite CBOR detected as single-test report"),
    }
    Ok(())
}

/// The tripwire that keeps try-both detection sound: the two formats
/// must stay structurally incompatible. If a field change ever makes
/// one deserialize as the other, auto-detection becomes ambiguous and
/// this fails first.
#[tokio::test]
async fn cross_type_deserialization_fails_tripwire() -> Result<()> {
    let mut suite_bytes = Vec::new();
    ciborium::into_writer(&make_sample_suite(), &mut suite_bytes)?;
    assert!(
        ciborium::from_reader::<TestReport, _>(&suite_bytes[..]).is_err(),
        "suite CBOR must not deserialize as TestReport"
    );

    let mut single_bytes = Vec::new();
    ciborium::into_writer(&make_sample_report(), &mut single_bytes)?;
    assert!(
        ciborium::from_reader::<SuiteReport, _>(&single_bytes[..]).is_err(),
        "single-test CBOR must not deserialize as SuiteReport"
    );
    Ok(())
}

#[tokio::test]
async fn import_any_rejects_schema_mismatch() -> Result<()> {
    let mut suite = make_sample_suite();
    suite.schema_version = REPORT_SCHEMA_VERSION - 1;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("old.cbor");
    export_report_cbor(&suite, &path).await?;

    // A structurally valid parse must surface the precise version
    // error, not fall through to "unrecognized format".
    match import_any_report_cbor(&path).await {
        Err(ImportError::SchemaVersion { found, expected }) => {
            assert_eq!(found, REPORT_SCHEMA_VERSION - 1);
            assert_eq!(expected, REPORT_SCHEMA_VERSION);
        }
        other => panic!("expected SchemaVersion error, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn import_any_rejects_garbage_with_both_causes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("garbage.cbor");
    tokio::fs::write(&path, b"this is not cbor at all").await?;

    match import_any_report_cbor(&path).await {
        Err(ImportError::UnrecognizedFormat { .. }) => {}
        other => panic!("expected UnrecognizedFormat error, got {other:?}"),
    }
    Ok(())
}
