//! `export_report`'s extension dispatch for suite reports: `.html`
//! renders, `.cbor`/extensionless stays raw CBOR (backward compatible),
//! anything else is rejected.

mod common;

use common::make_sample_suite;
use eyre::Result;
use speed_cli::utils::export::{ExportError, export_extension_supported, export_report};
use speed_cli::utils::import::import_suite_report_cbor;

#[tokio::test]
async fn suite_export_html_by_extension() -> Result<()> {
    let suite = make_sample_suite();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("suite.html");

    export_report(&suite, &path).await?;

    let html = tokio::fs::read_to_string(&path).await?;
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("Speed CLI Suite Report"));
    Ok(())
}

#[tokio::test]
async fn suite_export_extensionless_writes_cbor() -> Result<()> {
    let suite = make_sample_suite();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("suite-report");

    export_report(&suite, &path).await?;

    let decoded = import_suite_report_cbor(&path).await?;
    assert_eq!(decoded.reports.len(), suite.reports.len());
    Ok(())
}

#[tokio::test]
async fn suite_export_rejects_unknown_extension() -> Result<()> {
    let suite = make_sample_suite();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("suite.json");

    match export_report(&suite, &path).await {
        Err(ExportError::UnsupportedFormat(ext)) => assert_eq!(ext, "json"),
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
    // The preflight check agrees with the exporter.
    assert!(matches!(
        export_extension_supported(&path),
        Err(ExportError::UnsupportedFormat(_))
    ));
    assert!(export_extension_supported(std::path::Path::new("x.html")).is_ok());
    assert!(export_extension_supported(std::path::Path::new("x.cbor")).is_ok());
    assert!(export_extension_supported(std::path::Path::new("x")).is_ok());
    Ok(())
}
