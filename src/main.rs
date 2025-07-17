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

use crate::constants::{
    DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, DEFAULT_TCP_PORT, DEFAULT_UDP_PORT, MAX_HTTP_UPLOAD_SIZE,
};
use crate::performance::http::server::{HttpsServerConfig, run_https_server};
use crate::performance::http::{HttpVersion, client::run_http_test};
use crate::performance::tcp::server::run_tcp_server;
use crate::performance::udp::server::run_udp_server;
use crate::report::{HttpTestConfig, TcpTestConfig, TestReport, UdpTestConfig};
use crate::utils::export::{export_report, export_report_html};
use crate::utils::file::can_write;
use crate::utils::import::{import_report_cbor, import_report_json};
use crate::utils::progress::with_progress_counter;

mod cli;
mod constants;
mod performance;
mod renderer;
mod report;
mod utils;

/// Creates an optimized Tokio runtime for network performance testing
fn create_optimized_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_cpus::get())
        .thread_name("speed-cli-worker")
        .enable_all()
        .thread_stack_size(2 * 1024 * 1024) // 2MB stack size
        .build()
        .expect("Failed to create optimized Tokio runtime")
}

#[tokio::main(flavor = "multi_thread")]
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
            connections,
            test_sizes,
            test_type,
        } => {
            // Assert that exactly one specific protocol is enabled (no more, no less)
            // Count enabled protocols
            let protocols = [mode.is_some(), tcp, udp, http1, http2, h2c, http3];
            let protocol_count = protocols.iter().filter(|&&x| x).count();
            if protocol_count != 1 {
                return Err(eyre::eyre!(
                    "Exactly one protocol must be specified. Use --tcp, --udp, --http1, --http2, --h2c, or --http3."
                ));
            }

            let mode: ClientMode = mode.unwrap_or_else(|| {
                if tcp {
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
                    unreachable!()
                }
            });

            // Verify export file path is writable
            if let Some(export) = &export {
                if let Some(parent) = export.parent() {
                    fs::create_dir_all(parent)?;
                }
                if !can_write(export)? {
                    return Err(eyre::eyre!(
                        "Export file is not writable: {}",
                        export.display()
                    ));
                }
            }

            let report: TestReport = match mode {
                ClientMode::TCP => {
                    let config = TcpTestConfig::new(
                        server,
                        port,
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                    );

                    run_tcp_client(config).await?
                }
                ClientMode::UDP => {
                    let config = UdpTestConfig::new(
                        server,
                        port,
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                    );

                    run_udp_client(config).await?
                }
                ClientMode::HTTP1 | ClientMode::HTTP2 | ClientMode::H2C | ClientMode::HTTP3 => {
                    // For HTTP modes, we need to determine the HTTP version
                    let http_version = match mode {
                        ClientMode::HTTP1 => HttpVersion::HTTP1,
                        ClientMode::HTTP2 => HttpVersion::HTTP2,
                        ClientMode::H2C => HttpVersion::H2C,
                        ClientMode::HTTP3 => HttpVersion::HTTP3,
                        _ => unreachable!(),
                    };

                    let config = HttpTestConfig::new(
                        server,
                        port,
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                        http_version,
                    );

                    run_http_test(config).await?
                }
            };

            println!("{}", "Client test completed.".green().bold());

            // Print test report
            println!("{report:#}");

            // If export file is specified, write results
            if let Some(export) = &export {
                match with_progress_counter(
                    "Exporting test results",
                    export_report(&report, export),
                )
                .await
                {
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
            cert,
            key,
        } => {
            // Assert that at least one server mode is enabled
            if !all && !tcp && !udp && !http && !https {
                return Err(eyre::eyre!(
                    "At least one server mode must be enabled. Use --all to enable all modes."
                ));
            }

            // If HTTPS is enabled, cert and key should be provided or files exist at the default paths
            const FALLBACK_CERT_PATH: &str = "cert.pem";
            const FALLBACK_KEY_PATH: &str = "key.pem";
            if https || all {
                if cert.is_none() && !PathBuf::from(FALLBACK_CERT_PATH).exists() {
                    return Err(eyre::eyre!(
                        "HTTPS mode requires a TLS certificate. Provide --cert or ensure {FALLBACK_CERT_PATH} exists."
                    ));
                }
                if key.is_none() && !PathBuf::from(FALLBACK_KEY_PATH).exists() {
                    return Err(eyre::eyre!(
                        "HTTPS mode requires a TLS private key. Provide --key or ensure {FALLBACK_KEY_PATH} exists."
                    ));
                }
            }
            let cert = cert.unwrap_or(PathBuf::from(FALLBACK_CERT_PATH));
            let key = key.unwrap_or(PathBuf::from(FALLBACK_KEY_PATH));

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
                        enable_cors: true,
                        max_upload_size: MAX_HTTP_UPLOAD_SIZE,
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
                        enable_cors: true,
                        max_upload_size: MAX_HTTP_UPLOAD_SIZE,
                        cert_path: cert,
                        key_path: key,
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

        Commands::Report { file, export_html } => {
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
                        let report = with_progress_counter(
                            "Loading report from JSON file",
                            import_report_json(&file),
                        )
                        .await?;

                        match export_html {
                            None => {
                                // Print report in stdout
                                println!("{report:#}");
                            }
                            Some(html_file) => {
                                // Export to HTML
                                match with_progress_counter(
                                    "Exporting report to HTML",
                                    export_report_html(&report, &html_file),
                                )
                                .await
                                {
                                    Ok(_) => println!(
                                        "{}",
                                        format!("HTML report exported to {}", html_file.display())
                                            .cyan()
                                    ),
                                    Err(e) => eprintln!("Error exporting to HTML: {e}"),
                                }
                            }
                        }
                    }
                    "cbor" => {
                        let report = with_progress_counter(
                            "Loading report from CBOR file",
                            import_report_cbor(&file),
                        )
                        .await?;

                        match export_html {
                            None => {
                                // Print report in stdout
                                println!("{report:#}");
                            }
                            Some(html_file) => {
                                // Export to HTML
                                match with_progress_counter(
                                    "Exporting report to HTML",
                                    export_report_html(&report, &html_file),
                                )
                                .await
                                {
                                    Ok(_) => println!(
                                        "{}",
                                        format!("HTML report exported to {}", html_file.display())
                                            .cyan()
                                    ),
                                    Err(e) => eprintln!("Error exporting to HTML: {e}"),
                                }
                            }
                        }
                    }
                    "html" => {
                        return Err(eyre::eyre!(
                            "HTML report format should be opened via a web browser: {}",
                            file.display()
                        ));
                    }
                    _ => match with_progress_counter(
                        "Loading report from file (assuming JSON format)",
                        import_report_json(&file),
                    )
                    .await
                    {
                        Ok(report) => {
                            println!("{report:#}");
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
