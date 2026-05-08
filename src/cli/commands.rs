use std::{net::IpAddr, path::PathBuf};

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
        /// Server hostname or IP address (e.g., 182.168.1.1, google.com). Defaults to 127.0.0.1
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// Server port. If not specified, defaults to 5201 for TCP/UDP and 8080 for HTTP.
        #[arg(short, long)]
        port: Option<u16>,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        duration: u64,

        /// Warmup window in seconds. Samples taken in this initial window are
        /// discarded so the reported numbers reflect steady state. Counts
        /// against `--duration`, not on top of it.
        #[arg(long, default_value = "1")]
        warmup: u64,

        /// Protocol mode (tcp, udp, http1, http2, h2c, http3)
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

        /// Use HTTP/3 (QUIC)
        #[arg(long, alias = "quic")]
        #[clap(group = "protocol")]
        http3: bool,

        /// Export results to file (JSON, CBOR, or HTML depending on extension)
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
        /// (the default) means "saturate". Above ~100 Mbps the
        /// blaster's `tokio::time::sleep` pacing starts bunching
        /// packets; for those rates either accept the bursting or
        /// shape via `tc`.
        #[arg(long, default_value = "0")]
        target_rate_mbps: u64,

        /// Emit machine-readable JSON to stdout instead of the pretty
        /// text report.
        #[arg(long)]
        json: bool,
    },

    /// Run as server
    Server {
        /// Enable all server modes
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        #[clap(conflicts_with_all = ["tcp", "udp", "http", "https"])]
        all: bool,

        /// Enable TCP server mode
        #[arg(long)]
        tcp: bool,

        /// Enable UDP server mode
        #[arg(long)]
        udp: bool,

        /// Enable unencrypted HTTP server modes (i.e. HTTP/1.1 without TLS, h2c)
        #[arg(long, alias = "http1")]
        http: bool,

        /// Enable HTTPS server modes (i.e. HTTP/2, HTTP/3)
        #[arg(long)]
        https: bool,

        /// Bind to specific interface. Defaults to 0.0.0.0
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: IpAddr,

        /// Listen port for TCP server
        #[arg(long)]
        tcp_port: Option<u16>,

        /// Listen port for UDP server  
        #[arg(long)]
        udp_port: Option<u16>,

        /// Listen port for HTTP server
        #[arg(long)]
        http_port: Option<u16>,

        /// Listen port for HTTPS server
        #[arg(long)]
        https_port: Option<u16>,

        /// TLS certificate file path (*.pem).
        /// If not specified, defaults to `cert.pem` in the current directory.
        #[arg(long)]
        cert: Option<PathBuf>,

        /// TLS private key file path (*.pem).
        /// If not specified, defaults to `key.pem` in the current directory.
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Print previously saved results
    Report {
        /// Path to the results file (JSON or CBOR)
        #[arg(short, long)]
        file: PathBuf,

        /// Export results to HTML
        #[arg(long)]
        export_html: Option<PathBuf>,
    },

    /// Run the comprehensive end-to-end suite against a server. Drives
    /// every protocol speed-cli supports (TCP, UDP, HTTP/1, h2c,
    /// HTTP/2-TLS) and produces one combined report. HTTP/3 is skipped
    /// because the bundled server does not yet implement it.
    Suite {
        /// Server hostname or IP. Defaults to 127.0.0.1.
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// TCP/UDP port (servers expose both on the same port).
        #[arg(long, default_value = "5201")]
        tcp_udp_port: u16,

        /// HTTP (cleartext) port.
        #[arg(long, default_value = "8080")]
        http_port: u16,

        /// HTTPS port.
        #[arg(long, default_value = "8443")]
        https_port: u16,

        /// Duration *per phase* in seconds. Total wall-clock is roughly
        /// this × (number of phases) (~7 phases by default).
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

        /// Skip TLS phases (HTTPS/HTTP-2). Useful when the server
        /// wasn't started with `--https`.
        #[arg(long)]
        no_tls: bool,

        /// Goodput vs wire-rate accounting.
        #[arg(long, value_enum, default_value_t = AccountingArg::Goodput)]
        accounting: AccountingArg,

        /// Export the suite report to a file (JSON or CBOR).
        #[arg(short, long)]
        export: Option<PathBuf>,

        /// Emit machine-readable JSON to stdout instead of the pretty
        /// text report. Combine with `--export` to also save to disk.
        #[arg(long)]
        json: bool,
    },
}
