pub mod commands;

use clap::Parser;
pub use commands::*;

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
}
