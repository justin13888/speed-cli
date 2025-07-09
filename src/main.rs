use colored::*;
use eyre::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod cli;
mod network;
mod speed;
mod utils;

use cli::{Cli, Commands};
use speed::tcp::{TcpClientConfig, run_tcp_client};
use speed::udp::{UdpClientConfig, run_udp_client};
use speed::http::{HttpTestConfig, run_http_test};
use speed::http_server::{HttpServerConfig, run_http_server};
use speed::server::{ServerConfig, run_server};
use speed::diagnostics::{ComprehensiveTestConfig, run_comprehensive_test};
use tracing::debug;
use clap::Parser;

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
            
            if udp {
                let config = UdpClientConfig {
                    server_addr: server,
                    port,
                    duration: time,
                    target_bandwidth: bandwidth,
                    export_file: export,
                };
                run_udp_client(config).await?;
            } else {
                let config = TcpClientConfig {
                    server_addr: server,
                    port,
                    duration: time,
                    export_file: export,
                };
                run_tcp_client(config).await?;
            }
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
