use std::path::Path;

use crate::report::{REPORT_SCHEMA_VERSION, SuiteReport, TestReport};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("CBOR parsing error: {0}")]
    Cbor(#[from] ciborium::de::Error<std::io::Error>),
    #[error("schema version mismatch: file is v{found}, expected v{expected}")]
    SchemaVersion { found: u32, expected: u32 },
    #[error(
        "not a recognizable speed-cli report — as single-test report: {single}; as suite report: {suite}"
    )]
    UnrecognizedFormat {
        single: ciborium::de::Error<std::io::Error>,
        suite: ciborium::de::Error<std::io::Error>,
    },
}

/// A report loaded from disk whose kind was auto-detected. The two CBOR
/// layouts are structurally incompatible (disjoint required fields with
/// no serde defaults), so detection by trial deserialization is
/// deterministic — `tests/suite_roundtrip.rs` has a tripwire asserting
/// that stays true as the structs evolve.
#[derive(Debug)]
pub enum LoadedReport {
    Single(Box<TestReport>),
    Suite(Box<SuiteReport>),
}

fn check_schema(found: u32) -> Result<(), ImportError> {
    if found != REPORT_SCHEMA_VERSION {
        return Err(ImportError::SchemaVersion {
            found,
            expected: REPORT_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Read a CBOR-encoded `TestReport` from disk.
///
/// Reports whose `schema_version` does not match the current
/// `REPORT_SCHEMA_VERSION` are rejected: there is no migration path.
pub async fn import_report_cbor(filename: &Path) -> Result<TestReport, ImportError> {
    let content = tokio::fs::read(filename).await?;
    let report: TestReport = ciborium::from_reader(&content[..])?;
    check_schema(report.schema_version)?;
    Ok(report)
}

/// Read a CBOR-encoded `SuiteReport` from disk, with the same strict
/// schema-version gate as `import_report_cbor`.
pub async fn import_suite_report_cbor(filename: &Path) -> Result<SuiteReport, ImportError> {
    let content = tokio::fs::read(filename).await?;
    let report: SuiteReport = ciborium::from_reader(&content[..])?;
    check_schema(report.schema_version)?;
    Ok(report)
}

/// Read either kind of report, auto-detecting which one the file holds.
///
/// Tries `TestReport` first (the common case). A structural parse
/// success identifies the kind, so a schema-version mismatch is
/// reported immediately rather than falling through to the other
/// variant. Only when both parses fail structurally is the combined
/// `UnrecognizedFormat` returned.
pub async fn import_any_report_cbor(filename: &Path) -> Result<LoadedReport, ImportError> {
    let content = tokio::fs::read(filename).await?;

    let single_err = match ciborium::from_reader::<TestReport, _>(&content[..]) {
        Ok(report) => {
            check_schema(report.schema_version)?;
            return Ok(LoadedReport::Single(Box::new(report)));
        }
        Err(e) => e,
    };

    let suite_err = match ciborium::from_reader::<SuiteReport, _>(&content[..]) {
        Ok(report) => {
            check_schema(report.schema_version)?;
            return Ok(LoadedReport::Suite(Box::new(report)));
        }
        Err(e) => e,
    };

    Err(ImportError::UnrecognizedFormat {
        single: single_err,
        suite: suite_err,
    })
}
