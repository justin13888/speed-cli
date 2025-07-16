use std::path::Path;

use crate::report::TestReport;

use thiserror::Error;
#[derive(Debug, Error)]
pub enum ImportError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Attempts to parse a file as JSON.
pub async fn import_report_json(filename: &Path) -> Result<TestReport, ImportError> {
    let content = tokio::fs::read_to_string(filename).await?;
    Ok(serde_json::from_str(&content)?)
}
