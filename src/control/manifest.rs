//! The control manifest: the JSON document a speed-cli server publishes
//! at its control endpoint so a client can discover where every
//! per-protocol test listener lives and whether the two binaries speak
//! a compatible wire protocol.
//!
//! The manifest is the *only* thing a client needs to be told (host +
//! control port). Every test listener binds an OS-assigned ephemeral
//! port; the manifest maps each [`TestTransport`] to its real port.

use serde::{Deserialize, Serialize};

use crate::constants::PROTOCOL_VERSION;
use crate::report::{PeerIdentity, REPORT_SCHEMA_VERSION};

/// One test transport the server can expose. Each gets its own distinct
/// TCP or UDP port so a client can never accidentally drive the wrong
/// protocol against a shared port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestTransport {
    /// Raw TCP throughput (`'U'`/`'D'`/`'F'`/`'P'`/`'H'` byte commands).
    TcpRaw,
    /// UDP blaster.
    UdpBlaster,
    /// HTTP/1.1 cleartext.
    Http1,
    /// HTTP/2 cleartext (h2c, prior-knowledge).
    H2c,
    /// HTTP/2 over TLS.
    Http2Tls,
    /// HTTP/3 over QUIC.
    Http3,
    /// Raw QUIC stream throughput (QUIC analog of `TcpRaw`).
    QuicRaw,
}

impl TestTransport {
    /// Human-readable label used in logs and the manifest dump.
    pub fn label(&self) -> &'static str {
        match self {
            TestTransport::TcpRaw => "tcp",
            TestTransport::UdpBlaster => "udp",
            TestTransport::Http1 => "http1",
            TestTransport::H2c => "h2c",
            TestTransport::Http2Tls => "http2",
            TestTransport::Http3 => "http3",
            TestTransport::QuicRaw => "quic",
        }
    }
}

/// One advertised test listener.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerEntry {
    pub transport: TestTransport,
    /// Host the server bound on. Informational only — a client should
    /// dial the host it used to reach the control endpoint, because the
    /// server commonly binds `0.0.0.0` and cannot know its routable
    /// address.
    pub host: String,
    /// The real (often ephemeral) port the listener is bound to.
    pub port: u16,
}

/// JSON document served at `GET /` and `GET /manifest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerManifest {
    /// Gated. The client hard-errors unless this equals its own
    /// [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Informational: the report schema the server's binary writes.
    pub report_schema_version: u32,
    /// Informational: server binary version, git commit, os/arch, host.
    pub server_identity: PeerIdentity,
    /// Every enabled test listener and its port.
    pub listeners: Vec<ListenerEntry>,
}

impl ServerManifest {
    /// Build a manifest for the local server from a set of bound
    /// listeners.
    pub fn new(listeners: Vec<ListenerEntry>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            report_schema_version: REPORT_SCHEMA_VERSION,
            server_identity: PeerIdentity::local(),
            listeners,
        }
    }

    /// Look up the listener for a given transport, if advertised.
    pub fn listener(&self, transport: TestTransport) -> Option<&ListenerEntry> {
        self.listeners.iter().find(|l| l.transport == transport)
    }
}
