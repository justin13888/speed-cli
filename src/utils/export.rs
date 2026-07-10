use serde::Serialize;
use std::path::Path;
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::renderer::ToHtml;

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
/// data format is CBOR-only. Works for both single-test and suite
/// reports.
pub async fn export_report<T: Serialize + ToHtml>(
    report: &T,
    filename: &Path,
) -> Result<(), ExportError> {
    match filename.extension().and_then(|s| s.to_str()) {
        Some("html") => export_report_html(report, filename).await,
        Some("cbor") | None => export_report_cbor(report, filename).await,
        Some(other) => Err(ExportError::UnsupportedFormat(other.to_string())),
    }
}

/// True when `export_report` would accept this path. Callers that run a
/// long test before exporting should preflight with this so a typo'd
/// extension fails before the run, not after.
pub fn export_extension_supported(filename: &Path) -> Result<(), ExportError> {
    match filename.extension().and_then(|s| s.to_str()) {
        Some("html") | Some("cbor") | None => Ok(()),
        Some(other) => Err(ExportError::UnsupportedFormat(other.to_string())),
    }
}

pub async fn export_report_cbor<T: Serialize>(
    report: &T,
    filename: &Path,
) -> Result<(), ExportError> {
    let file = tokio::fs::File::create(filename).await?;
    let mut writer = BufWriter::new(file);

    let mut buffer = Vec::new();
    ciborium::into_writer(report, &mut buffer)?;
    writer.write_all(&buffer).await?;

    writer.flush().await?;
    Ok(())
}

/// Stream the rendered HTML straight to disk. Suite reports can embed
/// many phases, so this avoids materializing the whole document in
/// memory; `block_in_place` keeps the synchronous `Write` off the async
/// reactor (the runtime is multi-threaded).
pub async fn export_report_html<T: ToHtml>(report: &T, filename: &Path) -> Result<(), ExportError> {
    let render = || -> Result<(), ExportError> {
        use std::io::Write as _;
        let file = std::fs::File::create(filename)?;
        let mut writer = std::io::BufWriter::new(file);
        report.write_html(&mut writer)?;
        writer.flush()?;
        Ok(())
    };
    // block_in_place panics on a current-thread runtime (e.g. plain
    // #[tokio::test]); render inline there instead.
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(render)
        }
        _ => render(),
    }
}
