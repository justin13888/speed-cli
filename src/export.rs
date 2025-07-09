use csv::Writer;
use std::fs::File;
use std::path::Path;
use thiserror::Error;

use crate::network::TestResult;

#[derive(Debug, Error)]
pub enum ExportError {
    IO(#[from] std::io::Error),
    Serde(#[from] serde_json::Error),
    Csv(#[from] csv::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::IO(e) => write!(f, "I/O error: {e}"),
            ExportError::Serde(e) => write!(f, "Serialization error: {e}"),
            ExportError::Csv(e) => write!(f, "CSV error: {e}"),
        }
    }
}

pub async fn export_results(results: &[TestResult], filename: &str) -> Result<(), ExportError> {
    let path = Path::new(filename);
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    
    match extension.to_lowercase().as_str() {
        "json" => export_json(results, filename).await,
        "csv" => export_csv(results, filename).await,
        _ => {
            // Default to JSON if no extension or unknown extension
            let json_filename = format!("{filename}.json");
            export_json(results, &json_filename).await
        }
    }
}

async fn export_json(results: &[TestResult], filename: &str) -> Result<(), ExportError> {
    let json = serde_json::to_string_pretty(results)?;
    tokio::fs::write(filename, json).await?;
    Ok(())
}

async fn export_csv(results: &[TestResult], filename: &str) -> Result<(), ExportError> {
    let file = File::create(filename)?;
    let mut writer = Writer::from_writer(file);
    
    // Write header
    writer.write_record([
        "timestamp",
        "bytes_transferred", 
        "duration_seconds",
        "bandwidth_mbps",
        "jitter_ms",
        "packet_loss_percent"
    ])?;
    
    // Write data
    for result in results {
        writer.write_record(&[
            result.timestamp.to_rfc3339(),
            result.bytes_transferred.to_string(),
            result.duration.as_secs_f64().to_string(),
            result.bandwidth_mbps.to_string(),
            result.jitter_ms.map_or("".to_string(), |j| j.to_string()),
            result.packet_loss.map_or("".to_string(), |p| p.to_string()),
        ])?;
    }
    
    writer.flush()?;
    Ok(())
}
