//! Bind every enabled test listener up front, then hand back both the
//! spawn-ready listeners and the [`ListenerEntry`] list the control
//! manifest needs. Binding before serving is what lets the manifest
//! advertise real (often OS-assigned ephemeral) ports.

use std::net::{IpAddr, SocketAddr};

use eyre::{Context as _, Result};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::control::manifest::{ListenerEntry, TestTransport};
use crate::performance::http::h3_server::{Http3ServerConfig, bind_h3, run_h3_server};
use crate::performance::http::server::{
    HttpServerConfig, run_h2c_server, run_http1_server, run_https_server,
};
use crate::performance::quic::server::{QuicServerConfig, bind_quic, run_quic_server};
use crate::performance::tcp::server::run_tcp_server_on;
use crate::performance::udp::server::BlasterServer;
use crate::utils::tls::TlsMaterial;

/// Which test protocols to expose. `http` enables *both* the HTTP/1.1
/// and h2c listeners (separate ports each).
#[derive(Debug, Clone, Copy)]
pub struct EnabledProtocols {
    pub tcp: bool,
    pub udp: bool,
    pub http: bool,
    pub https: bool,
    pub http3: bool,
    pub quic: bool,
}

/// Optional fixed-port overrides. `None` => OS-assigned ephemeral port.
/// Kept for port-forwarded / firewalled deployments.
#[derive(Debug, Clone, Copy, Default)]
pub struct PortOverrides {
    pub tcp: Option<u16>,
    pub udp: Option<u16>,
    pub http1: Option<u16>,
    pub h2c: Option<u16>,
    pub https: Option<u16>,
    pub http3: Option<u16>,
    pub quic: Option<u16>,
}

/// Shared runtime knobs every test listener needs once it starts
/// serving.
pub struct ServerRuntime {
    pub bind: IpAddr,
    pub enable_cors: bool,
    pub max_upload_size: usize,
    pub buffer_size: usize,
    pub tls: TlsMaterial,
}

/// A test listener whose socket is already bound but not yet serving.
enum BoundListener {
    TcpRaw(TcpListener),
    UdpBlaster(Box<BlasterServer>),
    Http1(TcpListener),
    H2c(TcpListener),
    Http2Tls(std::net::TcpListener),
    Http3(quinn::Endpoint),
    QuicRaw(quinn::Endpoint),
}

/// The result of [`bind_all`]: spawn-ready listeners plus the manifest
/// entries describing their real ports.
pub struct BoundListeners {
    listeners: Vec<(TestTransport, BoundListener)>,
    pub entries: Vec<ListenerEntry>,
}

fn addr(bind: IpAddr, port: Option<u16>) -> SocketAddr {
    SocketAddr::new(bind, port.unwrap_or(0))
}

/// Bind every enabled listener. Each gets its own distinct port.
pub async fn bind_all(
    rt: &ServerRuntime,
    enabled: EnabledProtocols,
    overrides: PortOverrides,
) -> Result<BoundListeners> {
    let host = rt.bind.to_string();
    let mut listeners: Vec<(TestTransport, BoundListener)> = Vec::new();
    let mut entries: Vec<ListenerEntry> = Vec::new();

    let push = |transport: TestTransport,
                    port: u16,
                    bound: BoundListener,
                    listeners: &mut Vec<(TestTransport, BoundListener)>,
                    entries: &mut Vec<ListenerEntry>| {
        entries.push(ListenerEntry {
            transport,
            host: host.clone(),
            port,
        });
        listeners.push((transport, bound));
    };

    if enabled.tcp {
        let l = TcpListener::bind(addr(rt.bind, overrides.tcp))
            .await
            .wrap_err("binding TCP test listener")?;
        let port = l.local_addr()?.port();
        push(
            TestTransport::TcpRaw,
            port,
            BoundListener::TcpRaw(l),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.udp {
        let server = BlasterServer::new(addr(rt.bind, overrides.udp))
            .await
            .wrap_err("binding UDP blaster listener")?;
        let port = server.local_addr()?.port();
        push(
            TestTransport::UdpBlaster,
            port,
            BoundListener::UdpBlaster(Box::new(server)),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.http {
        let h1 = TcpListener::bind(addr(rt.bind, overrides.http1))
            .await
            .wrap_err("binding HTTP/1.1 test listener")?;
        let h1_port = h1.local_addr()?.port();
        push(
            TestTransport::Http1,
            h1_port,
            BoundListener::Http1(h1),
            &mut listeners,
            &mut entries,
        );

        let h2c = TcpListener::bind(addr(rt.bind, overrides.h2c))
            .await
            .wrap_err("binding h2c test listener")?;
        let h2c_port = h2c.local_addr()?.port();
        push(
            TestTransport::H2c,
            h2c_port,
            BoundListener::H2c(h2c),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.https {
        let l = std::net::TcpListener::bind(addr(rt.bind, overrides.https))
            .wrap_err("binding HTTPS test listener")?;
        let port = l.local_addr()?.port();
        push(
            TestTransport::Http2Tls,
            port,
            BoundListener::Http2Tls(l),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.http3 {
        let cfg = Http3ServerConfig {
            max_upload_size: rt.max_upload_size,
            tls: rt.tls.clone(),
        };
        let (endpoint, port) = bind_h3(addr(rt.bind, overrides.http3), &cfg)?;
        push(
            TestTransport::Http3,
            port,
            BoundListener::Http3(endpoint),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.quic {
        let cfg = QuicServerConfig {
            tls: rt.tls.clone(),
            buffer_size: rt.buffer_size,
        };
        let (endpoint, port) = bind_quic(addr(rt.bind, overrides.quic), &cfg)?;
        push(
            TestTransport::QuicRaw,
            port,
            BoundListener::QuicRaw(endpoint),
            &mut listeners,
            &mut entries,
        );
    }

    Ok(BoundListeners { listeners, entries })
}

impl BoundListeners {
    /// Spawn one task per bound listener. Returns `(label, handle)`
    /// pairs for the caller to join.
    pub fn spawn(
        self,
        rt: &ServerRuntime,
        cancel: &CancellationToken,
    ) -> Vec<(&'static str, JoinHandle<Result<()>>)> {
        let mut handles: Vec<(&'static str, JoinHandle<Result<()>>)> = Vec::new();

        for (transport, listener) in self.listeners {
            let cancel = cancel.clone();
            let enable_cors = rt.enable_cors;
            let max_upload_size = rt.max_upload_size;
            let buffer_size = rt.buffer_size;
            let tls = rt.tls.clone();

            let _ = transport;
            let (label, handle): (&'static str, JoinHandle<Result<()>>) = match listener {
                BoundListener::TcpRaw(l) => {
                    ("TCP", tokio::spawn(run_tcp_server_on(l, cancel)))
                }
                BoundListener::UdpBlaster(server) => (
                    "UDP",
                    tokio::spawn(async move { server.run(cancel).await }),
                ),
                BoundListener::Http1(l) => (
                    "HTTP/1.1",
                    tokio::spawn(run_http1_server(
                        l,
                        HttpServerConfig {
                            enable_cors,
                            max_upload_size,
                        },
                        cancel,
                    )),
                ),
                BoundListener::H2c(l) => (
                    "h2c",
                    tokio::spawn(run_h2c_server(
                        l,
                        HttpServerConfig {
                            enable_cors,
                            max_upload_size,
                        },
                        cancel,
                    )),
                ),
                BoundListener::Http2Tls(l) => (
                    "HTTPS",
                    tokio::spawn(async move {
                        let rustls = tls.axum_rustls_config()?;
                        run_https_server(l, rustls, enable_cors, max_upload_size, cancel).await
                    }),
                ),
                BoundListener::Http3(endpoint) => (
                    "HTTP/3",
                    tokio::spawn(run_h3_server(
                        endpoint,
                        Http3ServerConfig {
                            max_upload_size,
                            tls,
                        },
                        cancel,
                    )),
                ),
                BoundListener::QuicRaw(endpoint) => (
                    "QUIC",
                    tokio::spawn(run_quic_server(
                        endpoint,
                        QuicServerConfig { tls, buffer_size },
                        cancel,
                    )),
                ),
            };
            handles.push((label, handle));
        }

        handles
    }
}
