use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionError {
    /// Error related to connection establishment
    ConnectionFailed(String),
    /// Error related to data transfer
    TransferFailed(String),
    /// Error related to timeout
    Timeout(String),
    /// Custom error message
    Unknown(String),
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            ConnectionError::TransferFailed(msg) => write!(f, "Transfer failed: {}", msg),
            ConnectionError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            ConnectionError::Unknown(msg) => write!(f, "Unknown error: {}", msg),
        }
    }
}
