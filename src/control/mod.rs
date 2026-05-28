//! Control / handshake plane.
//!
//! A speed-cli server exposes a single JSON control endpoint on a
//! user-chosen port. It publishes a [`ServerManifest`] describing the
//! protocol-compatibility version and the (ephemeral) port of every
//! per-protocol test listener. Clients fetch the manifest first
//! ([`client::perform_handshake`]), verify version compatibility, and
//! only then dial the discovered ports — so a client never has to be
//! told more than one port and can never silently hit the wrong
//! protocol on a shared port.

pub mod bind;
pub mod client;
pub mod manifest;
pub mod server;

pub use bind::{EnabledProtocols, PortOverrides, ServerRuntime, bind_all};
pub use client::{Handshake, perform_handshake};
pub use manifest::{ServerManifest, TestTransport};
pub use server::{ControlServerConfig, run_control_server};
