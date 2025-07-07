use clap::{Parser, Subcommand};
use anyhow::Result;
use colored::*;

mod network;
mod client;
mod server;
mod export;
mod bandwidth;

use client::*;
use server::*;

#[derive(Parser)]
#[command(name = "speed-cli")]
#[command(about = "A network performance measurement tool (iperf3 clone)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as client (default mode)
    Client {
        /// Server hostname or IP address
        #[arg(short, long, default_value = "127.0.0.1")]
        server: String,
        
        /// Server port
        #[arg(short, long, default_value = "5201")]
        port: u16,
        
        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        time: u64,
        
        /// Use UDP instead of TCP
        #[arg(short, long)]
        udp: bool,
        
        /// Target bandwidth for UDP tests (Mbps)
        #[arg(short, long, default_value = "1")]
        bandwidth: f64,
        
        /// Export results to file (json or csv)
        #[arg(short, long)]
        export: Option<String>,
    },
    
    /// Run as server
    Server {
        /// Listen port
        #[arg(short, long, default_value = "5201")]
        port: u16,
        
        /// Bind to specific interface
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Client { server, port, time, udp, bandwidth, export } => {
            println!("{}", "Starting client mode...".green().bold());
            let config = ClientConfig {
                server_addr: server,
                port,
                duration: time,
                use_udp: udp,
                target_bandwidth: bandwidth,
                export_file: export,
            };
            run_client(config).await?;
        },
        
        Commands::Server { port, bind } => {
            println!("{}", "Starting server mode...".blue().bold());
            let config = ServerConfig {
                bind_addr: bind,
                port,
            };
            run_server(config).await?;
        },
    }
    
    Ok(())
}
