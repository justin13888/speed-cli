//! Logging and color setup.
//!
//! Results print to **stdout**; every log line, diagnostic, and progress
//! indicator goes to **stderr**, so `speed-cli client ... > results.cbor`
//! captures only the report and `2> logs.txt` captures only the noise.

use std::io::IsTerminal as _;

use clap::ValueEnum;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// When to colorize output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ColorChoice {
    /// Color when stderr is a terminal and `NO_COLOR` is unset (default).
    #[default]
    Auto,
    /// Always emit ANSI color.
    Always,
    /// Never emit color.
    Never,
}

impl ColorChoice {
    /// Resolve to a concrete on/off decision, honoring `NO_COLOR`.
    fn enabled(self) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => {
                std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
            }
        }
    }
}

/// Install the global tracing subscriber and color policy.
///
/// Verbosity precedence: if `RUST_LOG` is set it wins; otherwise `quiet`
/// selects `error`, and `verbose` raises the floor (`0 = info`, `1 = debug`,
/// `>= 2 = trace`).
pub fn init(verbose: u8, quiet: bool, color: ColorChoice) {
    let color_on = color.enabled();
    // Drives the `colored` crate used by the report Display impls (stdout).
    colored::control::set_override(color_on);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let level = if quiet {
            "error"
        } else if verbose >= 2 {
            "trace"
        } else if verbose == 1 {
            "debug"
        } else {
            "info"
        };
        let mut filter = EnvFilter::new(level);
        // reqwest's HTTP/3 pool logs the remote's graceful QUIC stream close
        // (RemoteTerminate code 0x0) at ERROR on every normal request teardown.
        // It is benign end-of-request behaviour, not a fault — real request
        // failures still surface via the returned Result. Suppress the noise by
        // default; `-vv` and RUST_LOG still reveal it.
        if verbose < 2 {
            filter = filter.add_directive(
                "reqwest::async_impl::h3_client::pool=off"
                    .parse()
                    .expect("static EnvFilter directive is valid"),
            );
        }
        filter
    });

    let base = tracing_subscriber::registry().with(filter);
    // Pretty multi-line output on a terminal; compact single-line for pipes,
    // logs, and CI.
    if std::io::stderr().is_terminal() {
        base.with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(color_on)
                .pretty(),
        )
        .init();
    } else {
        base.with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(color_on)
                .with_target(false)
                .compact(),
        )
        .init();
    }
}
