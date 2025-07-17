use serde::{Deserialize, Serialize};
use std::fmt;

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

impl HttpVersion {
    pub fn is_secure(&self) -> bool {
        matches!(self, HttpVersion::HTTP2 | HttpVersion::HTTP3)
    }

    /// Returns "http" or "https" based on the version.
    pub fn scheme(&self) -> &'static str {
        if self.is_secure() { "https" } else { "http" }
    }
}

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
