//! Control-handshake tests: a compatible manifest is accepted and its
//! listeners are discoverable; an incompatible `protocol_version` is a
//! hard error.

use std::sync::Arc;
use std::time::Duration;

use eyre::Result;
use speed_cli::constants::PROTOCOL_VERSION;
use speed_cli::control::manifest::{ListenerEntry, ServerManifest, TestTransport};
use speed_cli::control::{ControlServerConfig, perform_handshake, run_control_server};
use speed_cli::report::PeerIdentity;
use tokio_util::sync::CancellationToken;

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
    let manifest = ServerManifest::new(vec![ListenerEntry {
        transport: TestTransport::TcpRaw,
        host: "127.0.0.1".to_string(),
        port: 5555,
    }]);
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

#[tokio::test]
async fn handshake_rejects_incompatible_protocol_version() -> Result<()> {
    let manifest = ServerManifest {
        protocol_version: PROTOCOL_VERSION + 1,
        report_schema_version: 0,
        server_identity: PeerIdentity::local(),
        listeners: Vec::new(),
    };
    let (port, cancel) = serve(manifest).await?;

    let result = perform_handshake("127.0.0.1", port).await;
    assert!(
        result.is_err(),
        "handshake must reject a mismatched protocol_version"
    );

    cancel.cancel();
    Ok(())
}
