//! Bind every enabled test listener up front, then hand back both the
//! spawn-ready listeners and the [`ListenerEntry`] list the control
//! manifest needs. Binding before serving is what lets the manifest
//! advertise real (often OS-assigned ephemeral) ports.

use std::net::{IpAddr, SocketAddr};

use eyre::{Context as _, Result};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::CongestionAlgorithm;
use crate::control::manifest::{ListenerEntry, TestTransport};
use crate::performance::http::h3_server::{Http3ServerConfig, bind_h3, run_h3_server};
use crate::performance::http::server::{
    HttpServerConfig, run_h2c_server, run_http1_server, run_https_server,
};
use crate::performance::quic::server::{QuicServerConfig, bind_quic, run_quic_server};
use crate::performance::tcp::server::run_tcp_server_on;
use crate::performance::udp::server::BlasterServer;
use crate::utils::net::{bind_std_tcp_listener, bind_tcp_listener};
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
    /// The BBR variants of the two QUIC listeners (each QUIC protocol
    /// binds one endpoint per congestion controller).
    pub http3_bbr: Option<u16>,
    pub quic_bbr: Option<u16>,
}

/// Shared runtime knobs every test listener needs once it starts
/// serving.
pub struct ServerRuntime {
    pub bind: IpAddr,
    pub enable_cors: bool,
    pub max_upload_size: usize,
    pub buffer_size: usize,
    pub tls: TlsMaterial,
    /// Fixed SO_RCVBUF/SO_SNDBUF for test sockets, in bytes. `None`
    /// keeps kernel autotuning for TCP and the enlarged default for
    /// UDP. Listener-level for TCP: accepted sockets inherit it.
    pub socket_buffer: Option<usize>,
}

/// A test listener whose socket is already bound but not yet serving.
/// The QUIC variants remember which congestion controller was baked
/// into the endpoint at bind time, for labels and server config.
enum BoundListener {
    TcpRaw(TcpListener),
    UdpBlaster(Box<BlasterServer>),
    Http1(TcpListener),
    H2c(TcpListener),
    Http2Tls(std::net::TcpListener),
    Http3(quinn::Endpoint, CongestionAlgorithm),
    QuicRaw(quinn::Endpoint, CongestionAlgorithm),
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
                congestion: CongestionAlgorithm,
                bound: BoundListener,
                listeners: &mut Vec<(TestTransport, BoundListener)>,
                entries: &mut Vec<ListenerEntry>| {
        entries.push(ListenerEntry {
            transport,
            host: host.clone(),
            port,
            congestion,
        });
        listeners.push((transport, bound));
    };

    if enabled.tcp {
        let l = bind_tcp_listener(addr(rt.bind, overrides.tcp), rt.socket_buffer)
            .wrap_err("binding TCP test listener")?;
        let port = l.local_addr()?.port();
        push(
            TestTransport::TcpRaw,
            port,
            CongestionAlgorithm::default(),
            BoundListener::TcpRaw(l),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.udp {
        let server = BlasterServer::new(addr(rt.bind, overrides.udp), rt.socket_buffer)
            .await
            .wrap_err("binding UDP blaster listener")?;
        let port = server.local_addr()?.port();
        push(
            TestTransport::UdpBlaster,
            port,
            CongestionAlgorithm::default(),
            BoundListener::UdpBlaster(Box::new(server)),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.http {
        let h1 = bind_tcp_listener(addr(rt.bind, overrides.http1), rt.socket_buffer)
            .wrap_err("binding HTTP/1.1 test listener")?;
        let h1_port = h1.local_addr()?.port();
        push(
            TestTransport::Http1,
            h1_port,
            CongestionAlgorithm::default(),
            BoundListener::Http1(h1),
            &mut listeners,
            &mut entries,
        );

        let h2c = bind_tcp_listener(addr(rt.bind, overrides.h2c), rt.socket_buffer)
            .wrap_err("binding h2c test listener")?;
        let h2c_port = h2c.local_addr()?.port();
        push(
            TestTransport::H2c,
            h2c_port,
            CongestionAlgorithm::default(),
            BoundListener::H2c(h2c),
            &mut listeners,
            &mut entries,
        );
    }

    if enabled.https {
        let l = bind_std_tcp_listener(addr(rt.bind, overrides.https), rt.socket_buffer)
            .wrap_err("binding HTTPS test listener")?;
        let port = l.local_addr()?.port();
        push(
            TestTransport::Http2Tls,
            port,
            CongestionAlgorithm::default(),
            BoundListener::Http2Tls(l),
            &mut listeners,
            &mut entries,
        );
    }

    // The QUIC protocols bind one endpoint per congestion controller
    // and advertise both. Ordering contract: the cubic entry is pushed
    // first, so legacy first-match clients keep resolving cubic.
    if enabled.http3 {
        for (congestion, port_override) in [
            (CongestionAlgorithm::Cubic, overrides.http3),
            (CongestionAlgorithm::Bbr, overrides.http3_bbr),
        ] {
            let cfg = Http3ServerConfig {
                max_upload_size: rt.max_upload_size,
                tls: rt.tls.clone(),
                congestion,
            };
            let (endpoint, port) = bind_h3(addr(rt.bind, port_override), &cfg)?;
            push(
                TestTransport::Http3,
                port,
                congestion,
                BoundListener::Http3(endpoint, congestion),
                &mut listeners,
                &mut entries,
            );
        }
    }

    if enabled.quic {
        for (congestion, port_override) in [
            (CongestionAlgorithm::Cubic, overrides.quic),
            (CongestionAlgorithm::Bbr, overrides.quic_bbr),
        ] {
            let cfg = QuicServerConfig {
                tls: rt.tls.clone(),
                buffer_size: rt.buffer_size,
                congestion,
            };
            let (endpoint, port) = bind_quic(addr(rt.bind, port_override), &cfg)?;
            push(
                TestTransport::QuicRaw,
                port,
                congestion,
                BoundListener::QuicRaw(endpoint, congestion),
                &mut listeners,
                &mut entries,
            );
        }
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
                BoundListener::TcpRaw(l) => ("TCP", tokio::spawn(run_tcp_server_on(l, cancel))),
                BoundListener::UdpBlaster(server) => {
                    ("UDP", tokio::spawn(async move { server.run(cancel).await }))
                }
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
                BoundListener::Http3(endpoint, congestion) => (
                    match congestion {
                        CongestionAlgorithm::Cubic => "HTTP/3",
                        CongestionAlgorithm::Bbr => "HTTP/3 (bbr)",
                    },
                    tokio::spawn(run_h3_server(
                        endpoint,
                        Http3ServerConfig {
                            max_upload_size,
                            tls,
                            congestion,
                        },
                        cancel,
                    )),
                ),
                BoundListener::QuicRaw(endpoint, congestion) => (
                    match congestion {
                        CongestionAlgorithm::Cubic => "QUIC",
                        CongestionAlgorithm::Bbr => "QUIC (bbr)",
                    },
                    tokio::spawn(run_quic_server(
                        endpoint,
                        QuicServerConfig {
                            tls,
                            buffer_size,
                            congestion,
                        },
                        cancel,
                    )),
                ),
            };
            handles.push((label, handle));
        }

        handles
    }
}
