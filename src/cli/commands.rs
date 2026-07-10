use std::{net::IpAddr, path::PathBuf};

use crate::constants::DEFAULT_CONTROL_PORT;
use crate::{ClientMode, CongestionAlgorithm, TestType};
use clap::{Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AccountingArg {
    Goodput,
    Wire,
}

/// Protocols a server can serve. `Http` enables HTTP/1.1 and h2c; `Https` is
/// HTTP/2 over TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ServerProtocol {
    Tcp,
    Udp,
    Quic,
    Http,
    Https,
    Http3,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run as client
    Client {
        /// Server hostname or IP address (e.g., 192.168.1.1, google.com).
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// Control endpoint port. The client performs a handshake against
        /// this port to discover every per-protocol test listener and to
        /// verify wire-protocol compatibility before testing.
        #[arg(long, default_value_t = DEFAULT_CONTROL_PORT)]
        control_port: u16,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        duration: u64,

        /// Warmup window in seconds. Samples taken in this initial window are
        /// discarded so the reported numbers reflect steady state. Counts
        /// against `--duration`, not on top of it.
        #[arg(long, default_value = "1")]
        warmup: u64,

        /// Protocol to test: tcp, udp, quic, http1, h2c, http2, or http3.
        #[arg(short = 'p', long, value_enum)]
        protocol: ClientMode,

        /// Export results to file. `.cbor` (or no extension) writes the
        /// raw CBOR data report; `.html` writes a rendered single-file
        /// report. Other extensions are rejected.
        #[arg(short, long)]
        export: Option<PathBuf>,

        /// Number of concurrent connections or streams (means different things for different protocols)
        #[arg(short, long, default_value = "1")]
        connections: usize,

        /// Test type (download, upload, bidirectional, simultaneous, latency)
        #[arg(long = "type", default_value = "bidirectional")]
        test_type: TestType,

        /// Packet/payload sizes in bytes (e.g., 1024, 8192). If empty, uses default sizes.
        /// Note: TCP automatically segments anyways but this argument is kept for consistency.
        #[arg(long = "sizes", num_args = 0.., value_delimiter = ',')]
        test_sizes: Vec<usize>,

        /// Application-level chunk size for HTTP upload/download bodies (HTTP
        /// only; TCP and UDP ignore it). Visible on the wire mainly for
        /// HTTP/1.1 — HTTP/2 and HTTP/3 re-frame bodies into their own frame
        /// sizes regardless.
        #[arg(long)]
        chunk_size: Option<usize>,

        /// How throughput is reported: `goodput` (payload bytes only,
        /// default) or `wire` (adds an estimate of TCP/IP or UDP/IP
        /// framing overhead).
        #[arg(long, value_enum, default_value_t = AccountingArg::Goodput)]
        accounting: AccountingArg,

        /// Target send rate for UDP tests in megabits per second. 0
        /// (the default) means "saturate".
        #[arg(long, default_value = "0")]
        target_rate_mbps: u64,

        /// Congestion controller for the QUIC-based protocols (`-p quic`,
        /// `-p http3`), applied on both sides via the server's matching
        /// listener. TCP-based protocols always use the OS controller
        /// and reject `bbr`.
        #[arg(long, value_enum, default_value_t = CongestionAlgorithm::Cubic)]
        congestion: CongestionAlgorithm,
    },

    /// Run as server.
    ///
    /// The server publishes a single JSON control endpoint on
    /// `--control-port`. Every enabled test protocol binds its own
    /// distinct (OS-assigned ephemeral) port and is advertised through
    /// that endpoint; clients only ever need the control port.
    Server {
        /// Enable all server protocols.
        #[arg(short, long, action = clap::ArgAction::SetTrue, conflicts_with = "protocols")]
        all: bool,

        /// Protocols to serve (repeatable), e.g. `--protocol tcp --protocol http`.
        /// `http` serves HTTP/1.1 and h2c; `https` is HTTP/2 over TLS.
        #[arg(long = "protocol", value_enum)]
        protocols: Vec<ServerProtocol>,

        /// Bind to specific interface. Defaults to 0.0.0.0
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: IpAddr,

        /// Control / handshake endpoint port. The one port a user
        /// normally picks; everything else is auto-assigned.
        #[arg(long, default_value_t = DEFAULT_CONTROL_PORT)]
        control_port: u16,

        /// Fixed-port overrides (default: OS-assigned ephemeral). Kept
        /// for port-forwarded / firewalled deployments.
        #[arg(long)]
        tcp_port: Option<u16>,
        #[arg(long)]
        udp_port: Option<u16>,
        #[arg(long)]
        http1_port: Option<u16>,
        #[arg(long)]
        h2c_port: Option<u16>,
        #[arg(long)]
        https_port: Option<u16>,
        #[arg(long)]
        http3_port: Option<u16>,
        #[arg(long)]
        quic_port: Option<u16>,
        /// The QUIC protocols each bind a second, BBR-configured
        /// listener alongside the default CUBIC one; these pin those
        /// extra ports.
        #[arg(long)]
        http3_bbr_port: Option<u16>,
        #[arg(long)]
        quic_bbr_port: Option<u16>,

        /// TLS certificate file path (*.pem). Shared by the HTTPS,
        /// HTTP/3 and raw-QUIC listeners. A self-signed certificate is
        /// generated if omitted.
        #[arg(long)]
        cert: Option<PathBuf>,

        /// TLS private key file path (*.pem).
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Print previously saved results
    Report {
        /// Path to a saved results file: a single-test or suite CBOR
        /// export (auto-detected).
        #[arg(short, long)]
        file: PathBuf,

        /// Export results to HTML
        #[arg(long)]
        export_html: Option<PathBuf>,
    },

    /// Run the comprehensive end-to-end suite against a server. Performs
    /// the control handshake, then drives every protocol the server
    /// advertises — TCP, UDP, raw QUIC, HTTP/1.1, h2c, HTTP/2-TLS and
    /// HTTP/3 — producing one combined report.
    Suite {
        /// Server hostname or IP. Defaults to 127.0.0.1.
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// Control endpoint port. The suite handshakes here to discover
        /// every test listener.
        #[arg(long, default_value_t = DEFAULT_CONTROL_PORT)]
        control_port: u16,

        /// Duration *per phase* in seconds.
        #[arg(short, long, default_value = "8")]
        duration: u64,

        /// Warmup seconds inside each phase.
        #[arg(long, default_value = "1")]
        warmup: u64,

        /// Parallel connections / streams. When unset, auto-derives from the
        /// client's CPU cores (capped at 8).
        #[arg(short, long)]
        connections: Option<usize>,

        /// UDP target rate in Mbps for throughput phase. 0 = saturate.
        #[arg(long, default_value = "100")]
        udp_target_rate_mbps: u64,

        /// Skip TLS phases (HTTP/2-TLS, HTTP/3) even if the server
        /// advertises them.
        #[arg(long)]
        no_tls: bool,

        /// Goodput vs wire-rate accounting.
        #[arg(long, value_enum, default_value_t = AccountingArg::Goodput)]
        accounting: AccountingArg,

        /// Congestion controller for the raw-QUIC and HTTP/3 phases.
        /// One knob, not an extra matrix dimension: the suite runs the
        /// same phase count either way. TCP-based phases always use the
        /// OS controller.
        #[arg(long, value_enum, default_value_t = CongestionAlgorithm::Cubic)]
        congestion: CongestionAlgorithm,

        /// Export the suite report to file. `.cbor` (or no extension)
        /// writes the raw CBOR data report; `.html` writes a rendered
        /// single-file report. Other extensions are rejected.
        #[arg(short, long)]
        export: Option<PathBuf>,
    },

    /// Print a shell completion script to stdout (e.g. `completions zsh`).
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate man pages into a directory.
    Man {
        /// Output directory (created if missing).
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
    },
}
