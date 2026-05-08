use std::path::Path;
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::{renderer::ToHtml, report::TestReport};

#[derive(Debug, Error)]
pub enum ExportError {
    IO(#[from] std::io::Error),
    Cbor(#[from] ciborium::ser::Error<std::io::Error>),
    UnsupportedFormat(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::IO(e) => write!(f, "I/O error: {e}"),
            ExportError::Cbor(e) => write!(f, "CBOR serialization error: {e}"),
            ExportError::UnsupportedFormat(ext) => write!(
                f,
                "unsupported export format `.{ext}` — only `.cbor` (data) and `.html` (rendered) are supported"
            ),
        }
    }
}

/// Dispatches to the right exporter by extension.
///
/// `.cbor` (or no extension) → binary CBOR data export. `.html` →
/// rendered single-file report. Any other extension is an error: the
/// data format is CBOR-only.
pub async fn export_report(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    match filename.extension().and_then(|s| s.to_str()) {
        Some("html") => export_report_html(report, filename).await,
        Some("cbor") | None => export_report_cbor(report, filename).await,
        Some(other) => Err(ExportError::UnsupportedFormat(other.to_string())),
    }
}

pub async fn export_report_cbor(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    let file = tokio::fs::File::create(filename).await?;
    let mut writer = BufWriter::new(file);

    let mut buffer = Vec::new();
    ciborium::into_writer(report, &mut buffer)?;
    writer.write_all(&buffer).await?;

    writer.flush().await?;
    Ok(())
}

pub async fn export_report_html(report: &TestReport, filename: &Path) -> Result<(), ExportError> {
    let file = tokio::fs::File::create(filename).await?;
    let mut writer = BufWriter::new(file);

    let mut buffer = Vec::new();
    report.write_html(&mut buffer)?;

    writer.write_all(&buffer).await?;
    writer.flush().await?;

    Ok(())
}
