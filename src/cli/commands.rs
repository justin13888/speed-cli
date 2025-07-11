use std::{net::IpAddr, path::PathBuf};

use crate::{ClientMode, TestType};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Run as client (default mode)
    Client {
        /// Server hostname or IP address (e.g., 182.168.1.1, google.com). Defaults to 127.0.0.1
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// Server port. If not specified, defaults to 5201 for TCP/UDP and 8080 for HTTP.
        #[arg(short, long)]
        port: Option<u16>,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        time: u64,

        /// Protocol mode (tcp, udp, http)
        #[arg(short, long, default_value = "tcp", value_enum)]
        #[clap(group = "protocol")]
        mode: ClientMode,

        /// Use TCP protocol (default)
        #[arg(long, default_value = "true")]
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

        /// Export results to file (json or csv)
        #[arg(short, long)]
        export: Option<PathBuf>,

        /// Number of parallel connections
        #[arg(long, default_value = "1")]
        parallel: usize,

        /// Enable adaptive test sizing
        #[arg(long)]
        adaptive: bool,

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
        #[arg(long, default_value = "5201")]
        tcp_port: Option<u16>,

        /// Listen port for UDP server  
        #[arg(long, default_value = "5201")]
        udp_port: Option<u16>,

        /// Listen port for HTTP server
        #[arg(long, default_value = "8080")]
        http_port: Option<u16>,

        /// Listen port for HTTPS server
        #[arg(long, default_value = "8443")]
        https_port: Option<u16>,
    },

    // /// Run comprehensive network diagnostics
    // Diagnostics {
    //     /// Server URL for testing
    //     #[arg(short, long, default_value = "http://localhost:8080")]
    //     url: String,

    //     /// Test duration in seconds
    //     #[arg(short, long, default_value = "30")]
    //     time: u64,

    //     /// Number of parallel connections for HTTP tests
    //     #[arg(short, long, default_value = "4")]
    //     parallel: usize,

    //     /// Export results to file (json only)
    //     #[arg(short, long)]
    //     export: Option<String>,

    //     /// Skip DNS performance tests
    //     #[arg(long)]
    //     skip_dns: bool,

    //     /// Skip connection quality tests
    //     #[arg(long)]
    //     skip_quality: bool,

    //     /// Skip network topology analysis
    //     #[arg(long)]
    //     skip_topology: bool,
    // },
    /// Print previously saved results
    Report {
        /// Path to the results file (json or csv)
        #[arg(short, long)]
        file: PathBuf,
    },
}

impl Commands {
    pub fn validate(&self) -> Result<(), String> {
        if let Commands::Server {
            tcp_port,
            udp_port,
            http_port,
            https_port,
            tcp,
            udp,
            http,
            https,
            all,
            ..
        } = self
        {
            if tcp_port.is_some() && !tcp && !all {
                return Err("tcp_port can only be specified when tcp or all is enabled".to_string());
            }
            if udp_port.is_some() && !udp && !all {
                return Err("udp_port can only be specified when udp or all is enabled".to_string());
            }
            if http_port.is_some() && !http && !all {
                return Err(
                    "http_port can only be specified when http or all is enabled".to_string(),
                );
            }
            if https_port.is_some() && !https && !all {
                return Err(
                    "https_port can only be specified when https or all is enabled".to_string(),
                );
            }
        }
        Ok(())
    }
}
