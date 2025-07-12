pub mod client;
pub mod server;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpVersion {
    /// HTTP/1.1 without TLS
    HTTP1,
    /// HTTP/2 Cleartext (h2c)
    H2C,
    /// HTTP/2 with TLS
    HTTP2,
    /// HTTP/3 (QUIC)
    HTTP3,
}

use std::fmt;

use serde::{Deserialize, Serialize};

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpVersion::HTTP1 => write!(f, "HTTP/1.1"),
            HttpVersion::H2C => write!(f, "HTTP/2 Cleartext (h2c)"),
            HttpVersion::HTTP2 => write!(f, "HTTP/2 with TLS"),
            HttpVersion::HTTP3 => write!(f, "HTTP/3 (QUIC)"),
        }
    }
}
