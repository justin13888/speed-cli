use colored::*;
use eyre::{Result, WrapErr as _};
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, trace};

use clap::Parser;
use speed_cli::ClientMode;
use speed_cli::cli::{Cli, Commands};
use speed_cli::constants::MAX_HTTP_UPLOAD_SIZE;
use speed_cli::control::{
    ControlServerConfig, EnabledProtocols, PortOverrides, ServerManifest, ServerRuntime,
    TestTransport, bind_all, perform_handshake, run_control_server,
};
use speed_cli::performance::http::HttpVersion;
use speed_cli::performance::http::client::run_http_test;
use speed_cli::performance::quic::client::run_quic_client;
use speed_cli::performance::suite::{SuiteConfig, default_connections, run_suite};
use speed_cli::performance::tcp::client::run_tcp_client;
use speed_cli::performance::udp::client::run_udp_client;
use speed_cli::report::{
    DEFAULT_TCP_READ_BUFFER, HttpTestConfig, QuicTestConfig, TcpTestConfig, TestReport,
    UdpTestConfig,
};
use speed_cli::utils::export::{export_extension_supported, export_report, export_report_html};
use speed_cli::utils::file::can_write;
use speed_cli::utils::import::{LoadedReport, import_any_report_cbor};
use speed_cli::utils::progress::with_progress_counter;
use speed_cli::utils::tls::TlsMaterial;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    speed_cli::utils::logging::init(cli.verbose, cli.quiet, cli.color);
    speed_cli::performance::engine::progress::set_enabled(!cli.quiet);
    trace!("Parsed CLI arguments: {cli:#?}");

    match cli.command {
        Commands::Client {
            server,
            control_port,
            duration,
            warmup,
            protocol,
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
                speed_cli::cli::AccountingArg::Goodput => {
                    speed_cli::report::ThroughputAccounting::Goodput
                }
                speed_cli::cli::AccountingArg::Wire => {
                    speed_cli::report::ThroughputAccounting::Wire
                }
            };
            let target_rate_bps: u64 = target_rate_mbps.saturating_mul(1_000_000);

            let mode = protocol;

            // Report rows are keyed by payload size in insertion order; sort the
            // user-supplied sizes ascending so the report is deterministic
            // regardless of the order given on the command line. Empty stays
            // empty — defaults are applied downstream and are already ascending.
            let mut test_sizes = test_sizes;
            test_sizes.sort_unstable();

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
            protocols,
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
            use speed_cli::cli::ServerProtocol;
            let has = |p: ServerProtocol| all || protocols.contains(&p);
            let enabled = EnabledProtocols {
                tcp: has(ServerProtocol::Tcp),
                udp: has(ServerProtocol::Udp),
                http: has(ServerProtocol::Http),
                https: has(ServerProtocol::Https),
                http3: has(ServerProtocol::Http3),
                quic: has(ServerProtocol::Quic),
            };
            if !enabled.tcp
                && !enabled.udp
                && !enabled.http
                && !enabled.https
                && !enabled.http3
                && !enabled.quic
            {
                return Err(eyre::eyre!(
                    "At least one protocol must be enabled. Use --all or --protocol <p>."
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
                    // Single-test and suite CBOR files share the same
                    // extension; the importer detects which this is.
                    let loaded = with_progress_counter(
                        "Loading report from CBOR file",
                        import_any_report_cbor(&file),
                    )
                    .await?;
                    match export_html {
                        None => match &loaded {
                            LoadedReport::Single(report) => println!("{report:#}"),
                            LoadedReport::Suite(suite) => println!("{suite}"),
                        },
                        Some(html_file) => {
                            let export = async {
                                match &loaded {
                                    LoadedReport::Single(report) => {
                                        export_report_html(report.as_ref(), &html_file).await
                                    }
                                    LoadedReport::Suite(suite) => {
                                        export_report_html(suite.as_ref(), &html_file).await
                                    }
                                }
                            };
                            with_progress_counter("Exporting report to HTML", export)
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
            // Unset → auto-derive from the client's core count (capped).
            let connections = connections.unwrap_or_else(default_connections);
            let cfg = SuiteConfig {
                control_port,
                phase_duration: std::time::Duration::from_secs(duration),
                warmup: std::time::Duration::from_secs(warmup),
                connections,
                udp_target_rate_mbps,
                accounting: match accounting {
                    speed_cli::cli::AccountingArg::Goodput => {
                        speed_cli::report::ThroughputAccounting::Goodput
                    }
                    speed_cli::cli::AccountingArg::Wire => {
                        speed_cli::report::ThroughputAccounting::Wire
                    }
                },
                include_tls: !no_tls,
                // `server` plus the shared I/O-size defaults come from `new`.
                ..SuiteConfig::new(server)
            };

            if let Some(export) = &export {
                // Reject a typo'd extension now, not after a multi-minute run.
                export_extension_supported(export)?;
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
                export_report(&suite, export)
                    .await
                    .wrap_err_with(|| format!("exporting suite report to {}", export.display()))?;
                println!(
                    "{}",
                    format!("Suite report exported to {}", export.display()).cyan()
                );
            }
        }

        Commands::Completions { shell } => {
            use clap::CommandFactory as _;
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "speed-cli",
                &mut std::io::stdout(),
            );
        }

        Commands::Man { out_dir } => {
            use clap::CommandFactory as _;
            std::fs::create_dir_all(&out_dir)?;
            clap_mangen::generate_to(Cli::command(), &out_dir)?;
        }
    }

    Ok(())
}
