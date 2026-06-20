pub mod commands;

use clap::Parser;
pub use commands::*;

use crate::utils::logging::ColorChoice;

#[derive(Parser, Debug)]
#[command(name = "speed-cli")]
#[command(
    about = "A comprehensive network performance measurement tool for TCP-based, UDP-based, HTTP-based protocols"
)]
#[command(
    long_about = "A multi-protocol network performance tool. Measures throughput, latency, \
jitter, and packet loss over TCP, UDP, raw QUIC, and HTTP/1.1, h2c, HTTP/2 and HTTP/3 between \
two endpoints, and writes CBOR (re-importable) or HTML reports."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase log verbosity: -v = debug, -vv = trace. Overridden by RUST_LOG.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode: errors only, suppress progress. Overridden by RUST_LOG.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// When to use color: auto (default), always, or never. Honors NO_COLOR.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}
