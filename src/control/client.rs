//! Client side of the control handshake.
//!
//! Before running any test, a client fetches the server's
//! [`ServerManifest`] from its control endpoint and verifies that the
//! two binaries speak the same wire protocol. A version mismatch is a
//! hard error raised *before* any test runs, so the user gets a clear
//! message instead of a confusing failure deep inside a measurement.

use std::time::Duration;

use colored::Colorize as _;
use eyre::{Result, eyre};

use crate::constants::PROTOCOL_VERSION;
use crate::control::manifest::{ServerManifest, TestTransport};

/// Outcome of a successful handshake: the server's manifest plus the
/// host the client used to reach the control endpoint.
#[derive(Debug, Clone)]
pub struct Handshake {
    pub manifest: ServerManifest,
    /// Host the user gave (and the client reached the control endpoint
    /// on). Substituted for each listener's advertised `host` when
    /// dialing, because the server may have bound `0.0.0.0`.
    pub server_host: String,
}

impl Handshake {
    /// Resolve the `(host, port)` a client should dial for `transport`.
    /// Errors if the server did not advertise that transport.
    pub fn endpoint(&self, transport: TestTransport) -> Result<(String, u16)> {
        match self.manifest.listener(transport) {
            Some(entry) => Ok((self.server_host.clone(), entry.port)),
            None => Err(eyre!(
                "server does not expose the {} test listener (not in manifest)",
                transport.label()
            )),
        }
    }
}

/// Fetch and validate the control manifest. Hard-errors on a protocol
/// version mismatch.
pub async fn perform_handshake(server: &str, control_port: u16) -> Result<Handshake> {
    let url = format!("http://{server}:{control_port}/manifest");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| eyre!("failed to build control HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| eyre!("control handshake to {url} failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(eyre!(
            "control handshake to {url} returned HTTP {}",
            resp.status()
        ));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| eyre!("control handshake: reading manifest body failed: {e}"))?;

    let manifest: ServerManifest = serde_json::from_slice(&body)
        .map_err(|e| eyre!("control handshake: manifest is not valid JSON: {e}"))?;

    if manifest.protocol_version != PROTOCOL_VERSION {
        return Err(eyre!(
            "protocol version mismatch: server speaks v{}, this client speaks v{}. \
             Upgrade or downgrade one side so both match.",
            manifest.protocol_version,
            PROTOCOL_VERSION,
        ));
    }

    // Informational: surface who we're talking to and what is on offer.
    tracing::info!(
        "{}",
        format!(
            "Handshake OK — protocol v{} — server {}",
            manifest.protocol_version, manifest.server_identity
        )
        .green(),
    );
    let advertised: Vec<&str> = manifest
        .listeners
        .iter()
        .map(|l| l.transport.label())
        .collect();
    tracing::info!(
        "{}",
        format!("Server advertises: {}", advertised.join(", ")).bright_white()
    );

    Ok(Handshake {
        manifest,
        server_host: server.to_string(),
    })
}
