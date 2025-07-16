use std::path::Path;
use thiserror::Error;

use crate::{renderer::ToHtml, report::TestReport};

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

pub async fn export_report(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    match filename.extension() {
        Some(ext) if ext == "html" => export_report_html(report, filename).await,
        Some(ext) if ext == "json" => export_report_json(report, filename).await,
        _ => {
            println!(
                "No known extension detected in file path. Exporting to JSON format by default."
            );

            export_report_json(report, filename).await
        }
    }
}

pub async fn export_report_json(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    let json = serde_json::to_string_pretty(report)?;
    tokio::fs::write(filename, json).await?;
    Ok(())
}

pub async fn export_report_html(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    tokio::fs::write(filename, report.to_html()).await?;
    Ok(())
}
