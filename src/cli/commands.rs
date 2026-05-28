use std::{net::IpAddr, path::PathBuf};

use crate::constants::DEFAULT_CONTROL_PORT;
use crate::{ClientMode, TestType};
use clap::{Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AccountingArg {
    Goodput,
    Wire,
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

        /// Protocol mode (tcp, udp, quic, http1, http2, h2c, http3)
        #[arg(short, long, value_enum)]
        #[clap(group = "protocol")]
        mode: Option<ClientMode>,

        /// Use TCP protocol
        #[arg(long)]
        #[clap(group = "protocol")]
        tcp: bool,

        /// Use UDP protocol
        #[arg(long)]
        #[clap(group = "protocol")]
        udp: bool,

        /// Use raw QUIC stream throughput
        #[arg(long)]
        #[clap(group = "protocol")]
        quic: bool,

        /// Use HTTP/1.1 without TLS
        #[arg(long)]
        #[clap(group = "protocol", alias = "http")]
        http1: bool,

        /// Use HTTP/2 with TLS
        #[arg(long)]
        #[clap(group = "protocol")]
        http2: bool,

        /// Use h2c (HTTP/2 Cleartext)
        #[arg(long)]
        #[clap(group = "protocol")]
        h2c: bool,

        /// Use HTTP/3 (over QUIC)
        #[arg(long)]
        #[clap(group = "protocol")]
        http3: bool,

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

        /// Maximum chunk size. Effective only for HTTP/1.1 tests.
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
    },

    /// Run as server.
    ///
    /// The server publishes a single JSON control endpoint on
    /// `--control-port`. Every enabled test protocol binds its own
    /// distinct (OS-assigned ephemeral) port and is advertised through
    /// that endpoint; clients only ever need the control port.
    Server {
        /// Enable all server modes
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        #[clap(conflicts_with_all = ["tcp", "udp", "http", "https", "http3", "quic"])]
        all: bool,

        /// Enable TCP server mode
        #[arg(long)]
        tcp: bool,

        /// Enable UDP server mode
        #[arg(long)]
        udp: bool,

        /// Enable raw QUIC server mode
        #[arg(long)]
        quic: bool,

        /// Enable unencrypted HTTP server modes (HTTP/1.1 and h2c, on
        /// separate ports each)
        #[arg(long, alias = "http1")]
        http: bool,

        /// Enable HTTPS server mode (HTTP/2 over TLS)
        #[arg(long)]
        https: bool,

        /// Enable HTTP/3 server mode (over QUIC)
        #[arg(long)]
        http3: bool,

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
        /// Path to the results file (CBOR).
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

        /// Parallel connections / streams.
        #[arg(short, long, default_value = "4")]
        connections: usize,

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

        /// Export the suite report to a CBOR file.
        #[arg(short, long)]
        export: Option<PathBuf>,
    },
}
