use colored::*;
use eyre::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::utils::format::{format_bytes, format_throughput};

pub async fn run_tcp_server(addr: impl ToSocketAddrs) -> Result<()> {
    let listener = TcpListener::bind(&addr).await?;
    println!("{}", "TCP server ready to accept connections...".green());

    let connection_id = Arc::new(AtomicU64::new(0));

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("New TCP connection from {}", addr.to_string().cyan());

        let connection_id = connection_id.fetch_add(1, Ordering::Relaxed);
        let handler = Arc::new(OptimizedTcpHandler::new(connection_id as usize));

        tokio::spawn({
            let handler = handler.clone();
            async move {
                let mut buffer = vec![0u8; 8192];

                loop {
                    match socket.read(&mut buffer).await {
                        Ok(0) => {
                            // Connection closed
                            let (total_bytes, duration, throughput_mbps) = handler.get_stats();

                            println!(
                                "TCP connection from {} closed. {} received in {:.2}s ({})",
                                addr,
                                format_bytes(total_bytes).yellow(),
                                duration.as_secs_f64(),
                                format_throughput(throughput_mbps).green()
                            );
                            break;
                        }
                        Ok(n) => {
                            handler.add_bytes(n as u64);

                            // Report progress if necessary
                            if handler.should_report() {
                                let (total_bytes, _, throughput_mbps) = handler.get_stats();

                                println!(
                                    "TCP {}: {} received, {} throughput",
                                    addr,
                                    format_bytes(total_bytes).yellow(),
                                    format_throughput(throughput_mbps).green()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("TCP connection error from {addr}: {e}");
                            break;
                        }
                    }
                }
            }
        });
    }
}

// Optimized connection state to reduce allocations
#[derive(Debug)]
struct OptimizedTcpHandler {
    total_bytes: AtomicU64,
    start_time: Instant,
    last_report: std::sync::Mutex<Instant>,
    connection_id: usize,
}

impl OptimizedTcpHandler {
    fn new(connection_id: usize) -> Self {
        let now = Instant::now();
        Self {
            total_bytes: AtomicU64::new(0),
            start_time: now,
            last_report: std::sync::Mutex::new(now),
            connection_id,
        }
    }

    fn add_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn should_report(&self) -> bool {
        let mut last_report = self.last_report.lock().unwrap();
        if last_report.elapsed() >= Duration::from_secs(5) {
            *last_report = Instant::now();
            true
        } else {
            false
        }
    }

    fn get_stats(&self) -> (u64, Duration, f64) {
        let bytes = self.total_bytes.load(Ordering::Relaxed);
        let duration = self.start_time.elapsed();
        let throughput_mbps = (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0);
        (bytes, duration, throughput_mbps)
    }
}
