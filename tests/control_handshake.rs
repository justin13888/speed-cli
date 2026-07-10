//! Control-handshake tests: a compatible manifest is accepted and its
//! listeners are discoverable; an incompatible `protocol_version` is a
//! hard error.

use std::sync::Arc;
use std::time::Duration;

use eyre::Result;
use speed_cli::CongestionAlgorithm;
use speed_cli::constants::PROTOCOL_VERSION;
use speed_cli::control::manifest::{ListenerEntry, ServerManifest, TestTransport};
use speed_cli::control::{ControlServerConfig, perform_handshake, run_control_server};
use speed_cli::report::PeerIdentity;
use tokio_util::sync::CancellationToken;

fn entry(transport: TestTransport, port: u16, congestion: CongestionAlgorithm) -> ListenerEntry {
    ListenerEntry {
        transport,
        host: "127.0.0.1".to_string(),
        port,
        congestion,
    }
}

async fn pick_port() -> Result<u16> {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    Ok(l.local_addr()?.port())
}

async fn serve(manifest: ServerManifest) -> Result<(u16, CancellationToken)> {
    let port = pick_port().await?;
    let cancel = CancellationToken::new();
    let server_cancel = cancel.clone();
    tokio::spawn(async move {
        let _ = run_control_server(
            ControlServerConfig {
                bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
                manifest: Arc::new(manifest),
            },
            server_cancel,
        )
        .await;
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok((port, cancel))
}

#[tokio::test]
async fn handshake_accepts_compatible_manifest() -> Result<()> {
    let manifest = ServerManifest::new(vec![entry(
        TestTransport::TcpRaw,
        5555,
        CongestionAlgorithm::Cubic,
    )]);
    let (port, cancel) = serve(manifest).await?;

    let handshake = perform_handshake("127.0.0.1", port).await?;
    assert_eq!(handshake.manifest.protocol_version, PROTOCOL_VERSION);
    let (host, listener_port) = handshake.endpoint(TestTransport::TcpRaw)?;
    assert_eq!(host, "127.0.0.1");
    assert_eq!(listener_port, 5555);
    assert!(handshake.endpoint(TestTransport::QuicRaw).is_err());

    cancel.cancel();
    Ok(())
}

/// The dual-listener contract: both congestion variants are advertised
/// and resolvable, and a legacy first-match `endpoint()` lookup keeps
/// resolving the cubic port (the manifest lists cubic first).
#[tokio::test]
async fn manifest_advertises_cubic_and_bbr_quic_listeners() -> Result<()> {
    let manifest = ServerManifest::new(vec![
        entry(TestTransport::QuicRaw, 6001, CongestionAlgorithm::Cubic),
        entry(TestTransport::QuicRaw, 6002, CongestionAlgorithm::Bbr),
        entry(TestTransport::Http3, 6003, CongestionAlgorithm::Cubic),
        entry(TestTransport::Http3, 6004, CongestionAlgorithm::Bbr),
    ]);
    let (port, cancel) = serve(manifest).await?;

    let handshake = perform_handshake("127.0.0.1", port).await?;
    let (_, cubic_port) =
        handshake.endpoint_with(TestTransport::QuicRaw, CongestionAlgorithm::Cubic)?;
    let (_, bbr_port) =
        handshake.endpoint_with(TestTransport::QuicRaw, CongestionAlgorithm::Bbr)?;
    assert_eq!(cubic_port, 6001);
    assert_eq!(bbr_port, 6002);
    assert_ne!(cubic_port, bbr_port);

    // Legacy first-match resolution == cubic.
    let (_, legacy_port) = handshake.endpoint(TestTransport::QuicRaw)?;
    assert_eq!(legacy_port, cubic_port);

    let (_, h3_bbr) = handshake.endpoint_with(TestTransport::Http3, CongestionAlgorithm::Bbr)?;
    assert_eq!(h3_bbr, 6004);

    cancel.cancel();
    Ok(())
}

/// A manifest from an old server carries no `congestion` field; it must
/// deserialize as cubic so cubic lookups keep working.
#[test]
fn listener_entry_without_congestion_field_defaults_to_cubic() {
    let json = r#"{"transport":"quic-raw","host":"127.0.0.1","port":7001}"#;
    let entry: ListenerEntry = serde_json::from_str(json).expect("legacy entry must deserialize");
    assert_eq!(entry.congestion, CongestionAlgorithm::Cubic);
}

/// Asking an old / cubic-only server for BBR must be a clear error, not
/// a silent cubic test.
#[tokio::test]
async fn bbr_endpoint_missing_is_a_clear_error() -> Result<()> {
    let manifest = ServerManifest::new(vec![entry(
        TestTransport::QuicRaw,
        6001,
        CongestionAlgorithm::Cubic,
    )]);
    let (port, cancel) = serve(manifest).await?;

    let handshake = perform_handshake("127.0.0.1", port).await?;
    let err = handshake
        .endpoint_with(TestTransport::QuicRaw, CongestionAlgorithm::Bbr)
        .expect_err("bbr lookup against a cubic-only manifest must fail");
    assert!(
        err.to_string().contains("bbr"),
        "error should name the missing algorithm, got: {err}"
    );

    cancel.cancel();
    Ok(())
}

#[tokio::test]
async fn handshake_rejects_incompatible_protocol_version() -> Result<()> {
    let manifest = ServerManifest {
        protocol_version: PROTOCOL_VERSION + 1,
        report_schema_version: 0,
        server_identity: PeerIdentity::local(),
        listeners: Vec::new(),
    };
    let (port, cancel) = serve(manifest).await?;

    let err = perform_handshake("127.0.0.1", port)
        .await
        .expect_err("handshake must reject a mismatched protocol_version");
    // Assert it rejected *because of* the version, not some incidental error.
    assert!(
        err.to_string().contains("protocol version mismatch"),
        "error should identify the protocol version mismatch, got: {err}"
    );

    cancel.cancel();
    Ok(())
}
