use std::{net::IpAddr, path::PathBuf};

use crate::{ClientMode, TestType};
use clap::Subcommand;

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

        /// Export results to file (JSON or HTML depending on extension)
        #[arg(short, long)]
        export: Option<PathBuf>,

        /// Number of concurrent connections or streams (means different things for different protocols)
        #[arg(short, long, default_value = "1")]
        connections: usize,

        /// Packet/payload sizes in bytes (e.g., 1024, 8192). If empty, uses default sizes.
        /// Note: TCP automatically segments anyways but this argument is kept for consistency.
        #[arg(long = "sizes", num_args = 0.., value_delimiter = ',')]
        test_sizes: Vec<usize>,

        /// Test type (download, upload, bidirectional, simultaneous, latency)
        #[arg(long = "type", default_value = "bidirectional")]
        test_type: TestType,

        /// Enable debug output
        #[arg(long)]
        debug: bool,
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
        /// Path to the results file (only JSON)
        #[arg(short, long)]
        file: PathBuf,

        /// Export results to HTML
        #[arg(long)]
        export_html: Option<PathBuf>,
    },
}
