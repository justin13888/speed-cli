use std::path::Path;

use crate::report::{REPORT_SCHEMA_VERSION, TestReport};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("CBOR parsing error: {0}")]
    Cbor(#[from] ciborium::de::Error<std::io::Error>),
    #[error("schema version mismatch: file is v{found}, expected v{expected}")]
    SchemaVersion { found: u32, expected: u32 },
}

/// Read a CBOR-encoded `TestReport` from disk.
///
/// Reports whose `schema_version` does not match the current
/// `REPORT_SCHEMA_VERSION` are rejected: there is no migration path.
pub async fn import_report_cbor(filename: &Path) -> Result<TestReport, ImportError> {
    let content = tokio::fs::read(filename).await?;
    let report: TestReport = ciborium::from_reader(&content[..])?;
    if report.schema_version != REPORT_SCHEMA_VERSION {
        return Err(ImportError::SchemaVersion {
            found: report.schema_version,
            expected: REPORT_SCHEMA_VERSION,
        });
    }
    Ok(report)
}
