use std::path::Path;
use thiserror::Error;

use crate::report::TestReport;

#[derive(Debug, Error)]
pub enum ExportError {
    IO(#[from] std::io::Error),
    Serde(#[from] serde_json::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::IO(e) => write!(f, "I/O error: {e}"),
            ExportError::Serde(e) => write!(f, "Serialization error: {e}"),
        }
    }
}

pub async fn export_report(reports: &[TestReport], filename: &Path) -> Result<(), ExportError> {
    match filename.extension() {
        Some(ext) if ext == "html" => {
            todo!("Exporting to HTML is not yet implemented");
        }
        Some(ext) if ext == "json" => {
            // JSON export
            export_report_json(reports, filename).await
        }
        _ => {
            println!(
                "No known extension detected in file path. Exporting to JSON format by default."
            );

            export_report_json(reports, filename).await
        }
    }
}

async fn export_report_json(reports: &[TestReport], filename: &Path) -> Result<(), ExportError> {
    let json = serde_json::to_string_pretty(reports)?;
    tokio::fs::write(filename, json).await?;
    Ok(())
}
