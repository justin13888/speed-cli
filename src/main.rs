use colored::*;
use eyre::Result;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing::trace;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use clap::Parser;
use cli::{Cli, Commands};
use performance::http::server::{HttpServerConfig, run_http_server};
use performance::tcp::client::run_tcp_client;
use performance::udp::client::run_udp_client;

pub use utils::types::*;

use crate::constants::{DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DEFAULT_TCP_PORT, DEFAULT_UDP_PORT};
use crate::performance::http::server::{HttpsServerConfig, run_https_server};
use crate::performance::http::{HttpVersion, client::run_http_test};
use crate::performance::tcp::server::run_tcp_server;
use crate::performance::udp::server::run_udp_server;
use crate::report::{HttpTestConfig, TcpTestConfig, TestReport, UdpTestConfig};
use crate::utils::export::export_report;
use crate::utils::file::can_write;
use crate::utils::import::import_report;

mod cli;
mod constants;
mod performance;
mod report;
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

    trace!("Parsed CLI arguments: {:#?}", cli);

    match cli.command {
        Commands::Client {
            server,
            port,
            duration,
            mode,
            tcp,
            udp,
            http1,
            http2,
            h2c,
            http3,
            export,
            parallel,
            test_sizes,
            test_type,
            debug,
        } => {
            // Assert that exactly one specific protocol is enabled (no more, no less)
            let mut protocol_count = 0;
            if tcp {
                protocol_count += 1;
            }
            if udp {
                protocol_count += 1;
            }
            if http1 {
                protocol_count += 1;
            }
            if http2 {
                protocol_count += 1;
            }
            if h2c {
                protocol_count += 1;
            }
            if http3 {
                protocol_count += 1;
            }
            if protocol_count != 1 {
                return Err(eyre::eyre!(
                    "Exactly one protocol must be specified. Use --tcp, --udp, --http1, --http2, --h2c, or --http3."
                ));
            }
            let mode: ClientMode = if tcp {
                ClientMode::TCP
            } else if udp {
                ClientMode::UDP
            } else if http1 {
                ClientMode::HTTP1
            } else if http2 {
                ClientMode::HTTP2
            } else if h2c {
                ClientMode::H2C
            } else if http3 {
                ClientMode::HTTP3
            } else {
                unreachable!();
            };

            println!("{}", "Starting client mode...".green().bold());

            // TODO: Do something about debug flag...
            // TODO: if debug on, debug log everything (config, test progress verbosely, etc.)

            // Verify export file path is writable
            // TODO: Validate this logic via unit tests
            if let Some(export) = &export {
                // Create parent directory if it doesn't exist
                if let Some(parent) = export.parent()
                    && !parent.exists()
                {
                    match fs::create_dir_all(parent) {
                        Ok(_) => {
                            // println!("Parent directory created or already exists.")
                        }
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
                            // println!("Export file is writable: {}", export.display());
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

            let mut reports: Vec<TestReport> = vec![];

            match mode {
                ClientMode::TCP => {
                    let config = TcpTestConfig::new(server, port, duration, parallel, test_sizes);
                    let tcp_report = run_tcp_client(config).await?;
                    reports.push(tcp_report);
                }
                ClientMode::UDP => {
                    let config = UdpTestConfig::new(server, port, duration, parallel, test_sizes);

                    let udp_report = run_udp_client(config).await?;
                    reports.push(udp_report);
                }
                ClientMode::HTTP1 => {
                    let config = HttpTestConfig::new(
                        server,
                        port,
                        duration,
                        parallel,
                        test_type,
                        test_sizes,
                        HttpVersion::HTTP1,
                    );

                    let http_report = run_http_test(config).await?;
                    reports.push(http_report);
                }
                ClientMode::HTTP2 => {
                    let config = HttpTestConfig::new(
                        server,
                        port,
                        duration,
                        parallel,
                        test_type,
                        test_sizes,
                        HttpVersion::HTTP2,
                    );

                    let http_report = run_http_test(config).await?;
                    reports.push(http_report);
                }
                ClientMode::H2C => todo!(),
                ClientMode::HTTP3 => todo!(),
            }

            println!("{}", "Client test completed.".green().bold());

            // Print test reports
            for report in &reports {
                println!("{:#}", report);
            }

            // If export file is specified, write results
            if let Some(export) = &export {
                match export_report(&reports, export).await {
                    Ok(_) => println!(
                        "{}",
                        format!("Results exported to {}", export.to_string_lossy()).cyan()
                    ),
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
                let tcp_addr = SocketAddr::new(bind, tcp_port.unwrap_or(DEFAULT_TCP_PORT));
                handles.push(("TCP", tokio::spawn(run_tcp_server(tcp_addr))));
            }

            // Setup UDP
            if all || udp {
                let udp_addr = SocketAddr::new(bind, udp_port.unwrap_or(DEFAULT_UDP_PORT));
                handles.push(("UDP", tokio::spawn(run_udp_server(udp_addr))));
            }

            // Setup HTTP server modes (i.e. HTTP/1.1 without TLS, h2c)
            if all || http {
                let http_addr = SocketAddr::new(bind, http_port.unwrap_or(DEFAULT_HTTP_PORT));

                handles.push((
                    "HTTPS",
                    tokio::spawn(run_http_server(HttpServerConfig {
                        bind_addr: http_addr,
                        enable_cors: true, // Always enable CORS as clients typically are at unexpected origins
                        max_upload_size: 10 * 1024 * 1024, // 10MB
                    })),
                ));
            }

            // Setup HTTPS server modes (i.e. HTTP/2, HTTP/3)
            if all || https {
                let https_addr = SocketAddr::new(bind, https_port.unwrap_or(DEFAULT_HTTPS_PORT));

                handles.push((
                    "HTTP",
                    tokio::spawn(run_https_server(HttpsServerConfig {
                        bind_addr: https_addr,
                        enable_cors: true, // Always enable CORS as clients typically are at unexpected origins
                        max_upload_size: 10 * 1024 * 1024, // 10MB
                        cert_path: PathBuf::from("cert.pem"), // TODO: Replace with actual cert path
                        key_path: PathBuf::from("key.pem"), // TODO: Replace with actual key
                    })),
                ));
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
            if let Some(ext) = file.extension() {
                match ext.to_string_lossy().as_ref() {
                    "json" => {
                        let reports = import_report(&file).await?;
                        for report in reports {
                            println!("{report:#}");
                        }
                    }
                    "html" => {
                        return Err(eyre::eyre!(
                            "HTML report format should be opened via a web browser: {}",
                            file.display()
                        ));
                    }
                    _ => match import_report(&file).await {
                        Ok(reports) => {
                            for report in reports {
                                println!("{report:#}");
                            }
                        }
                        Err(e) => {
                            eprintln!("Error parsing report (assumed to be JSON): {e}");
                        }
                    },
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
