use clap::{Parser, Subcommand};
use colored::*;
use eyre::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod bandwidth;
mod client;
mod diagnostics;
mod export;
mod http;
mod http_server;
mod network;
mod server;

use client::*;
use diagnostics::*;
use http::*;
use http_server::*;
use server::*;
use tracing::debug;

#[derive(Parser)]
#[command(name = "speed-cli")]
#[command(
    about = "A comprehensive network performance measurement tool (iperf3 + HTTP speed tests)"
)]
#[command(
    long_about = "A comprehensive network diagnostics tool that includes:\n• Traditional TCP/UDP throughput testing (like iperf3)\n• HTTP/1.1 and HTTP/2 speed tests (like Ookla/Cloudflare)\n• DNS performance analysis\n• Connection quality assessment\n• Network topology analysis\n• Geographic information and routing analysis"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// TODO: Review definition and make it stricter
#[derive(Subcommand)]
enum Commands {
    /// Run as client (default mode)
    Client {
        /// Server hostname or IP address
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,

        /// Server port
        #[arg(short, long, default_value = "5201")]
        port: u16,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        time: u64,

        /// Use UDP instead of TCP
        #[arg(short, long)]
        udp: bool,

        /// Target bandwidth for UDP tests (Mbps)
        #[arg(short, long, default_value = "1")]
        bandwidth: f64,

        /// Export results to file (json or csv)
        #[arg(short, long)]
        export: Option<String>,
    },

    /// Run as server
    Server {
        /// Listen port
        #[arg(short, long, default_value = "5201")]
        port: u16,

        /// Bind to specific interface
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,
    },

    /// Run HTTP speed test (similar to Ookla)
    Http {
        /// Server URL (e.g., http://localhost:8080)
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        time: u64,

        /// Number of parallel connections
        #[arg(short, long, default_value = "4")]
        parallel: usize,

        /// HTTP version (http1, http2, auto)
        #[arg(long, default_value = "auto")]
        version: HttpVersion,

        /// Test type (download, upload, bidirectional, latency, comprehensive)
        #[arg(long = "type", default_value = "comprehensive")]
        test_type: HttpTestType,

        /// Export results to file (json or csv)
        #[arg(short, long)]
        export: Option<String>,

        /// Enable adaptive test sizing
        #[arg(long)]
        adaptive: bool,
    },

    /// Run HTTP speed test server
    HttpServer {
        /// Listen port
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Bind to specific interface
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,

        /// Maximum upload size in MB
        #[arg(long, default_value = "100")]
        max_upload_mb: usize,

        /// Enable CORS headers
        #[arg(long, default_value = "true")]
        cors: bool,
    },

    /// Run comprehensive network diagnostics
    Diagnostics {
        /// Server URL for testing
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,

        /// Test duration in seconds
        #[arg(short, long, default_value = "30")]
        time: u64,

        /// Number of parallel connections for HTTP tests
        #[arg(short, long, default_value = "4")]
        parallel: usize,

        /// Export results to file (json only)
        #[arg(short, long)]
        export: Option<String>,

        /// Skip DNS performance tests
        #[arg(long)]
        skip_dns: bool,

        /// Skip connection quality tests
        #[arg(long)]
        skip_quality: bool,

        /// Skip network topology analysis
        #[arg(long)]
        skip_topology: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing subscriber for logging
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();
    let fmt_layer = fmt::layer()
        .pretty()
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    // Start parsing
    let cli = Cli::parse();

    match cli.command {
        Commands::Client {
            server,
            port,
            time,
            udp,
            bandwidth,
            export,
        } => {
            println!("{}", "Starting client mode...".green().bold());
            let config = ClientConfig {
                server_addr: server,
                port,
                duration: time,
                use_udp: udp,
                target_bandwidth: bandwidth,
                export_file: export,
            };
            run_client(config).await?;
        }

        Commands::Server { port, bind } => {
            println!("{}", "Starting server mode...".blue().bold());
            let config = ServerConfig {
                bind_addr: bind,
                port,
            };
            run_server(config).await?;
        }

        Commands::Http {
            url,
            time,
            parallel,
            version,
            test_type,
            export,
            adaptive,
        } => {
            println!("{}", "Starting HTTP speed test...".green().bold());

            let config = HttpTestConfig {
                server_url: url,
                duration: time,
                parallel_connections: parallel,
                test_type,
                http_version: version,
                test_sizes: vec![1024 * 1024, 10 * 1024 * 1024, 100 * 1024 * 1024], // 1MB, 10MB, 100MB
                adaptive_sizing: adaptive,
                export_file: export,
            };

            debug!("HTTP Test Config: {:?}", config);

            run_http_test(config).await?;
        }

        Commands::HttpServer {
            port,
            bind,
            max_upload_mb,
            cors,
        } => {
            println!("{}", "Starting HTTP speed test server...".blue().bold());
            let config = HttpServerConfig {
                bind_addr: bind,
                port,
                enable_cors: cors,
                max_upload_size: max_upload_mb * 1024 * 1024, // Convert MB to bytes
                static_files_path: None,
            };
            run_http_server(config).await?;
        }

        Commands::Diagnostics {
            url,
            time,
            parallel,
            export,
            skip_dns,
            skip_quality,
            skip_topology,
        } => {
            println!(
                "{}",
                "Starting comprehensive network diagnostics..."
                    .green()
                    .bold()
            );

            let config = ComprehensiveTestConfig {
                server_url: url,
                test_duration: time,
                include_dns_tests: !skip_dns,
                include_quality_tests: !skip_quality,
                include_topology_tests: !skip_topology,
                parallel_connections: parallel,
                export_file: export,
            };

            run_comprehensive_test(config).await?;
        }
    }

    Ok(())
}
