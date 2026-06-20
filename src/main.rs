use colored::*;
use eyre::{Result, WrapErr as _};
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, trace};

use clap::Parser;
use cli::{Cli, Commands};
use performance::suite::{SuiteConfig, run_suite};
use performance::tcp::client::run_tcp_client;
use performance::udp::client::run_udp_client;

pub use utils::types::*;

use crate::constants::MAX_HTTP_UPLOAD_SIZE;
use crate::control::{
    ControlServerConfig, EnabledProtocols, PortOverrides, ServerManifest, ServerRuntime,
    TestTransport, bind_all, perform_handshake, run_control_server,
};
use crate::performance::http::HttpVersion;
use crate::performance::http::client::run_http_test;
use crate::performance::quic::client::run_quic_client;
use crate::report::{
    DEFAULT_TCP_READ_BUFFER, HttpTestConfig, QuicTestConfig, TcpTestConfig, TestReport,
    UdpTestConfig,
};
use crate::utils::export::{export_report, export_report_html};
use crate::utils::file::can_write;
use crate::utils::import::import_report_cbor;
use crate::utils::progress::with_progress_counter;
use crate::utils::tls::TlsMaterial;

mod cli;
mod constants;
mod control;
mod performance;
mod renderer;
mod report;
mod utils;

/// Creates an optimized Tokio runtime for network performance testing
#[allow(dead_code)]
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
    let cli = Cli::parse();
    utils::logging::init(cli.verbose, cli.quiet, cli.color);
    performance::engine::progress::set_enabled(!cli.quiet);
    trace!("Parsed CLI arguments: {cli:#?}");

    match cli.command {
        Commands::Client {
            server,
            control_port,
            duration,
            warmup,
            mode,
            tcp,
            udp,
            quic,
            http1,
            http2,
            h2c,
            http3,
            export,
            connections,
            test_type,
            test_sizes,
            chunk_size,
            accounting,
            target_rate_mbps,
        } => {
            if warmup >= duration {
                return Err(eyre::eyre!(
                    "--warmup ({warmup}s) must be less than --duration ({duration}s)"
                ));
            }
            let warmup = std::time::Duration::from_secs(warmup);
            let accounting = match accounting {
                cli::AccountingArg::Goodput => crate::report::ThroughputAccounting::Goodput,
                cli::AccountingArg::Wire => crate::report::ThroughputAccounting::Wire,
            };
            let target_rate_bps: u64 = target_rate_mbps.saturating_mul(1_000_000);

            // Exactly one protocol must be selected.
            let protocols = [mode.is_some(), tcp, udp, quic, http1, http2, h2c, http3];
            if protocols.iter().filter(|&&x| x).count() != 1 {
                return Err(eyre::eyre!(
                    "Exactly one protocol must be specified. Use --tcp, --udp, --quic, --http1, --http2, --h2c, or --http3."
                ));
            }
            let mode: ClientMode = mode.unwrap_or({
                if tcp {
                    ClientMode::TCP
                } else if udp {
                    ClientMode::UDP
                } else if quic {
                    ClientMode::QUIC
                } else if http1 {
                    ClientMode::HTTP1
                } else if http2 {
                    ClientMode::HTTP2
                } else if h2c {
                    ClientMode::H2C
                } else {
                    ClientMode::HTTP3
                }
            });

            // Verify export file path is writable.
            if let Some(export) = &export {
                if let Some(parent) = export.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                if !can_write(export)? {
                    return Err(eyre::eyre!(
                        "Export file is not writable: {}",
                        export.display()
                    ));
                }
            }

            // Control handshake: discover ports + verify compatibility.
            let handshake = perform_handshake(&server, control_port).await?;

            let transport = match mode {
                ClientMode::TCP => TestTransport::TcpRaw,
                ClientMode::UDP => TestTransport::UdpBlaster,
                ClientMode::QUIC => TestTransport::QuicRaw,
                ClientMode::HTTP1 => TestTransport::Http1,
                ClientMode::H2C => TestTransport::H2c,
                ClientMode::HTTP2 => TestTransport::Http2Tls,
                ClientMode::HTTP3 => TestTransport::Http3,
            };
            let (host, port) = handshake.endpoint(transport)?;

            let report: TestReport = match mode {
                ClientMode::TCP => {
                    let config = TcpTestConfig::new(
                        host,
                        Some(port),
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                    )
                    .with_warmup(warmup)
                    .with_accounting(accounting);
                    run_tcp_client(config).await?
                }
                ClientMode::UDP => {
                    let config = UdpTestConfig::new(
                        host,
                        Some(port),
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                    )
                    .with_warmup(warmup)
                    .with_accounting(accounting)
                    .with_target_rate_bps(target_rate_bps);
                    run_udp_client(config).await?
                }
                ClientMode::QUIC => {
                    let config = QuicTestConfig::new(
                        host,
                        Some(port),
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                    )
                    .with_warmup(warmup)
                    .with_accounting(accounting);
                    run_quic_client(config).await?
                }
                ClientMode::HTTP1 | ClientMode::HTTP2 | ClientMode::H2C | ClientMode::HTTP3 => {
                    let http_version = match mode {
                        ClientMode::HTTP1 => HttpVersion::HTTP1,
                        ClientMode::HTTP2 => HttpVersion::HTTP2,
                        ClientMode::H2C => HttpVersion::H2C,
                        ClientMode::HTTP3 => HttpVersion::HTTP3,
                        _ => unreachable!(),
                    };
                    let config = HttpTestConfig::new(
                        host,
                        Some(port),
                        duration,
                        connections,
                        test_type,
                        test_sizes,
                        chunk_size,
                        http_version,
                    )
                    .with_warmup(warmup)
                    .with_accounting(accounting);
                    run_http_test(config).await?
                }
            };

            println!("{}", "Client test completed.".green().bold());
            println!("{report:#}");

            if let Some(export) = &export {
                with_progress_counter("Exporting test results", export_report(&report, export))
                    .await
                    .wrap_err_with(|| format!("exporting results to {}", export.display()))?;
                println!(
                    "{}",
                    format!("Results exported to {}", export.to_string_lossy()).cyan()
                );
            }
        }

        Commands::Server {
            all,
            tcp,
            udp,
            quic,
            http,
            https,
            http3,
            bind,
            control_port,
            tcp_port,
            udp_port,
            http1_port,
            h2c_port,
            https_port,
            http3_port,
            quic_port,
            cert,
            key,
        } => {
            let enabled = EnabledProtocols {
                tcp: tcp || all,
                udp: udp || all,
                http: http || all,
                https: https || all,
                http3: http3 || all,
                quic: quic || all,
            };
            if !enabled.tcp
                && !enabled.udp
                && !enabled.http
                && !enabled.https
                && !enabled.http3
                && !enabled.quic
            {
                return Err(eyre::eyre!(
                    "At least one server mode must be enabled. Use --all to enable all modes."
                ));
            }

            println!("{}", "Starting server mode...".blue().bold());

            // Resolve TLS material once; shared by HTTPS, HTTP/3, raw QUIC.
            let tls = match (cert, key) {
                (Some(cert), Some(key)) => {
                    if !cert.exists() || !key.exists() {
                        return Err(eyre::eyre!(
                            "Certificate and key files must exist: {} and {}",
                            cert.display(),
                            key.display()
                        ));
                    }
                    TlsMaterial::from_pem_files(&cert, &key)?
                }
                (None, None) => TlsMaterial::self_signed()?,
                _ => {
                    return Err(eyre::eyre!(
                        "Both --cert and --key must be specified together"
                    ));
                }
            };

            let cancel = CancellationToken::new();
            let rt = ServerRuntime {
                bind,
                enable_cors: true,
                max_upload_size: MAX_HTTP_UPLOAD_SIZE,
                buffer_size: DEFAULT_TCP_READ_BUFFER,
                tls,
            };
            let overrides = PortOverrides {
                tcp: tcp_port,
                udp: udp_port,
                http1: http1_port,
                h2c: h2c_port,
                https: https_port,
                http3: http3_port,
                quic: quic_port,
            };

            // Bind every test listener up front so the manifest carries
            // the real (often ephemeral) ports.
            let bound = bind_all(&rt, enabled, overrides).await?;
            let manifest = Arc::new(ServerManifest::new(bound.entries.clone()));

            println!(
                "{}",
                format!("Control endpoint: http://{bind}:{control_port}/manifest")
                    .green()
                    .bold()
            );
            for entry in &manifest.listeners {
                println!(
                    "  {:<7} -> {}:{}",
                    entry.transport.label().bright_white().bold(),
                    bind,
                    entry.port.to_string().yellow()
                );
            }

            let mut handles = bound.spawn(&rt, &cancel);

            // Control endpoint last, once the manifest is final.
            let control_addr = SocketAddr::new(bind, control_port);
            handles.push((
                "Control",
                tokio::spawn(run_control_server(
                    ControlServerConfig {
                        bind_addr: control_addr,
                        manifest,
                    },
                    cancel.clone(),
                )),
            ));

            // Signal handler: SIGINT (all platforms), SIGTERM on Unix.
            let cancel_for_signal = cancel.clone();
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    let mut sigterm = match signal(SignalKind::terminate()) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Failed to install SIGTERM handler: {e}");
                            return;
                        }
                    };
                    tokio::select! {
                        res = tokio::signal::ctrl_c() => {
                            if let Err(e) = res {
                                error!("ctrl_c handler error: {e}");
                                return;
                            }
                            println!("\n{}", "Received SIGINT, shutting down gracefully...".yellow().bold());
                        }
                        _ = sigterm.recv() => {
                            println!("\n{}", "Received SIGTERM, shutting down gracefully...".yellow().bold());
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    if let Err(e) = tokio::signal::ctrl_c().await {
                        error!("ctrl_c handler error: {e}");
                        return;
                    }
                    println!(
                        "\n{}",
                        "Received SIGINT, shutting down gracefully..."
                            .yellow()
                            .bold()
                    );
                }
                cancel_for_signal.cancel();
            });

            println!(
                "{}",
                format!(
                    "Started servers: {}",
                    handles
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .blue()
                .bold()
            );

            let results = futures::future::join_all(
                handles
                    .into_iter()
                    .map(|(name, handle)| async move { (name, handle.await) }),
            )
            .await;

            let mut any_failed = false;
            for (name, result) in results {
                match result {
                    Ok(Ok(())) => info!("{name} server completed successfully"),
                    Ok(Err(e)) => {
                        error!("{name} server failed: {e}");
                        any_failed = true;
                    }
                    Err(e) => {
                        error!("{name} server task panicked: {e}");
                        any_failed = true;
                    }
                }
            }
            if any_failed {
                return Err(eyre::eyre!("one or more server listeners failed"));
            }
        }

        Commands::Report { file, export_html } => {
            if !file.exists() {
                return Err(eyre::eyre!(
                    "Report file does not exist: {}",
                    file.display()
                ));
            }
            if !file.is_file() {
                return Err(eyre::eyre!("Report path is not a file: {}", file.display()));
            }
            let ext = file
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            match ext.as_deref() {
                Some("html") => {
                    return Err(eyre::eyre!(
                        "HTML reports are for browsers, not re-import: {}",
                        file.display()
                    ));
                }
                Some("cbor") | None => {
                    let report = with_progress_counter(
                        "Loading report from CBOR file",
                        import_report_cbor(&file),
                    )
                    .await?;
                    match export_html {
                        None => println!("{report:#}"),
                        Some(html_file) => {
                            with_progress_counter(
                                "Exporting report to HTML",
                                export_report_html(&report, &html_file),
                            )
                            .await
                            .wrap_err_with(|| {
                                format!("exporting HTML report to {}", html_file.display())
                            })?;
                            println!(
                                "{}",
                                format!("HTML report exported to {}", html_file.display()).cyan()
                            );
                        }
                    }
                }
                Some(other) => {
                    return Err(eyre::eyre!(
                        "Unsupported report extension `.{other}`: only `.cbor` is accepted ({})",
                        file.display()
                    ));
                }
            }
        }

        Commands::Suite {
            server,
            control_port,
            duration,
            warmup,
            connections,
            udp_target_rate_mbps,
            no_tls,
            accounting,
            export,
        } => {
            if warmup >= duration {
                return Err(eyre::eyre!(
                    "--warmup ({warmup}s) must be less than per-phase --duration ({duration}s)"
                ));
            }
            let cfg = SuiteConfig {
                control_port,
                phase_duration: std::time::Duration::from_secs(duration),
                warmup: std::time::Duration::from_secs(warmup),
                connections,
                udp_target_rate_mbps,
                accounting: match accounting {
                    cli::AccountingArg::Goodput => crate::report::ThroughputAccounting::Goodput,
                    cli::AccountingArg::Wire => crate::report::ThroughputAccounting::Wire,
                },
                include_tls: !no_tls,
                // `server` plus the shared I/O-size defaults come from `new`.
                ..SuiteConfig::new(server)
            };

            if let Some(export) = &export {
                if let Some(parent) = export.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                if !can_write(export)? {
                    return Err(eyre::eyre!(
                        "Export file is not writable: {}",
                        export.display()
                    ));
                }
            }

            let suite = run_suite(cfg).await?;

            println!("{}", "Suite completed.".green().bold());
            println!("{suite}");

            if let Some(export) = &export {
                let mut buf = Vec::new();
                ciborium::into_writer(&suite, &mut buf)
                    .map_err(|e| eyre::eyre!("CBOR encode: {e}"))?;
                tokio::fs::write(export, &buf).await?;
                println!(
                    "{}",
                    format!("Suite report exported to {}", export.display()).cyan()
                );
            }
        }
    }

    Ok(())
}
