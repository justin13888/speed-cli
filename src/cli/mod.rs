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
    long_about = "A comprehensive network diagnostics tool that includes:\n• Traditional TCP/UDP throughput testing (like iperf3)\n• HTTP/1.1 and HTTP/2 speed tests (like Ookla/Cloudflare)\n• DNS performance analysis\n• Connection quality assessment\n• Network topology analysis\n• Geographic information and routing analysis"
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
