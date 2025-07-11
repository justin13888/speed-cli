use colored::*;
use eyre::Result;
use std::net::SocketAddr;
use std::{fs, io};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use clap::Parser;
use cli::{Cli, Commands};
use speed::http::HttpTestConfig;
use speed::http_server::{HttpServerConfig, run_http_server};
use speed::tcp::{TcpClientConfig, run_tcp_client};
use speed::udp::{UdpClientConfig, run_udp_client};
use tracing::debug;

pub use utils::types::*;

use crate::speed::http::run_http_test;
use crate::speed::server::{run_tcp_server, run_udp_server};
use crate::utils::export::export_results;
use crate::utils::file::can_write;

mod cli;
mod network;
mod speed;
mod utils;

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

    // TODO: Make all constants (e.g. default ports) be offloaded to the `Commands` struct
    match cli.command {
        Commands::Client {
            server,
            port,
            time,
            mode,
            export,
            parallel,
            adaptive,
            test_type,
            debug,
            ..
        } => {
            println!("{}", "Starting client mode...".green().bold());

            // TODO: Do something about debug flag...

            // Verify export file path is writable
            // TODO: Validate this logic via unit tests
            if let Some(export) = &export {
                // Create parent directory if it doesn't exist
                if let Some(parent) = export.parent()
                    && !parent.exists()
                {
                    match fs::create_dir_all(parent) {
                        Ok(_) => println!("Parent directory created or already exists."),
                        Err(e) => {
                            eprintln!("Error creating parent directory: {e}");
                            return Err(e.into());
                        }
                    }
                }

                // Validate export file is writable
                match can_write(export) {
                    Ok(writeable) => {
                        if writeable {
                            println!("Export file is writable: {}", export.display());
                        } else {
                            return Err(eyre::eyre!(
                                "Export file is not writable: {}",
                                export.display()
                            ));
                        }
                    }
                    Err(e) => {
                        return Err(eyre::eyre!("Error checking export file writability: {e}"));
                    }
                }
            }

            let mut test_results = vec![];

            match mode {
                ClientMode::TCP => {
                    let config = TcpClientConfig {
                        server_addr: server,
                        port: port.unwrap_or(5201), // Default TCP port
                        duration: time,
                        export_file: export.clone(),
                    };
                    run_tcp_client(config).await?;
                }
                ClientMode::UDP => {
                    let config = UdpClientConfig {
                        server_addr: server,
                        port: port.unwrap_or(5201), // Default UDP port
                        duration: time,
                        target_bandwidth: 100.0, // TODO: get rid of this field
                        export_file: export.clone(),
                    }; // TODO: Verify all attributes are being used
                    run_udp_client(config).await?;
                }
                ClientMode::HTTP1 => {
                    println!("{}", "Starting HTTP speed test...".green().bold());

                    let config = HttpTestConfig {
                        server_url: format!("http://{}:{}", server, port.unwrap_or(8080)), // Default HTTP port
                        duration: time,
                        parallel_connections: parallel,
                        test_type,
                        http_version: speed::http::HttpVersion::HTTP1,
                        test_sizes: vec![1024 * 1024, 10 * 1024 * 1024, 100 * 1024 * 1024], // 1MB, 10MB, 100MB
                        adaptive_sizing: adaptive,
                        export_file: export.clone(),
                    };

                    run_http_test(config).await?;
                }
                ClientMode::HTTP2 => todo!(),
                ClientMode::H2C => todo!(),
                ClientMode::HTTP3 => todo!(),
            }

            println!("{}", "Client test completed.".green().bold());

            // If export file is specified, write results
            if let Some(export) = &export {
                // TODO: Write test results to file as JSON or CSV
                match export_results(&test_results, export).await {
                    Ok(_) => println!("Results exported to {}", export.display()),
                    Err(e) => eprintln!("Error exporting results: {e}"),
                }
            }
        }

        Commands::Server {
            all,
            tcp,
            udp,
            http,
            https,
            bind,
            tcp_port,
            udp_port,
            http_port,
            https_port,
        } => {
            // Assert that at least one server mode is enabled
            if !all && !tcp && !udp && !http && !https {
                return Err(eyre::eyre!(
                    "At least one server mode must be enabled. Use --all to enable all modes."
                ));
            }

            println!("{}", "Starting server mode...".blue().bold());

            let mut handles: Vec<(&str, tokio::task::JoinHandle<_>)> = vec![];

            // Setup TCP
            if all || tcp {
                let tcp_addr = SocketAddr::new(bind, tcp_port.unwrap_or(5201));
                handles.push(("TCP", tokio::spawn(run_tcp_server(tcp_addr))));
            }

            // Setup UDP
            if all || udp {
                let udp_addr = SocketAddr::new(bind, udp_port.unwrap_or(5201));
                handles.push(("UDP", tokio::spawn(run_udp_server(udp_addr))));
            }

            // Setup HTTP server modes (i.e. HTTP/1.1 without TLS, h2c)
            if all || http {
                let http_addr = SocketAddr::new(bind, http_port.unwrap_or(8080));

                handles.push((
                    "HTTP",
                    tokio::spawn(run_http_server(HttpServerConfig {
                        bind_addr: http_addr,
                        enable_cors: true, // Always enable CORS as clients typically are at unexpected origins
                        max_upload_size: 10 * 1024 * 1024, // 10MB
                        static_files_path: None,
                    })),
                ));
            }

            // Setup HTTPS server modes (i.e. HTTP/2, HTTP/3)
            if all || https {
                let https_addr = SocketAddr::new(bind, https_port.unwrap_or(8443));

                handles.push((
                    "HTTPS",
                    tokio::spawn(run_http_server(HttpServerConfig {
                        bind_addr: https_addr,
                        enable_cors: true, // Always enable CORS as clients typically are at unexpected origins
                        max_upload_size: 10 * 1024 * 1024, // 10MB
                        static_files_path: None,
                    })),
                )); // TODO: Replace this with actual HTTPS server logic
            }

            // Log servers to be startup
            println!(
                "{}",
                format!(
                    "Starting servers: {}",
                    handles
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .blue()
                .bold()
            );

            // Wait for all server tasks to complete
            let results = futures::future::join_all(
                handles
                    .into_iter()
                    .map(|(name, handle)| async move { (name, handle.await) }),
            )
            .await;

            for (name, result) in results {
                if let Err(e) = result {
                    eprintln!("{name} server task failed: {e}");
                }
            }
        }
        // Commands::Diagnostics {
        //     url,
        //     time,
        //     parallel,
        //     export,
        //     skip_dns,
        //     skip_quality,
        //     skip_topology,
        // } => {
        //     println!(
        //         "{}",
        //         "Starting comprehensive network diagnostics..."
        //             .green()
        //             .bold()
        //     );

        //     let config = ComprehensiveTestConfig {
        //         server_url: url,
        //         test_duration: time,
        //         include_dns_tests: !skip_dns,
        //         include_quality_tests: !skip_quality,
        //         include_topology_tests: !skip_topology,
        //         parallel_connections: parallel,
        //         export_file: export,
        //     };

        //     run_comprehensive_test(config).await?;
        // }
        Commands::Report { file } => {
            println!("{}", "Loading report...".yellow().bold());

            // Validate file exists and is readable
            if !file.exists() {
                return Err(eyre::eyre!(
                    "Report file does not exist: {}",
                    file.display()
                ));
            }
            if !file.is_file() {
                return Err(eyre::eyre!("Report path is not a file: {}", file.display()));
            }
            if !file.metadata()?.permissions().readonly() {
                return Err(eyre::eyre!(
                    "Report file is not readable: {}",
                    file.display()
                ));
            }
            if let Some(ext) = file.extension() {
                match ext.to_string_lossy().as_ref() {
                    "json" => todo!(), // Handle JSON report
                    "csv" => todo!(),  // Handle CSV report
                    _ => {
                        return Err(eyre::eyre!(
                            "Unsupported report file extension: {}",
                            ext.to_string_lossy()
                        ));
                    }
                }
            } else {
                return Err(eyre::eyre!(
                    "Report file must have an extension: {}",
                    file.display()
                ));
            }
        }
    }

    Ok(())
}
