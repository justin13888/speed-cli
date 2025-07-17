use colored::*;
use eyre::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, broadcast};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::utils::format::{format_bytes, format_throughput};

// TODO: Try pushing this to 100gig connection

#[derive(Debug, Clone)]
pub struct TcpServerConfig {
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Connection timeout duration
    pub connection_timeout: Duration,
    /// Read timeout duration
    pub read_timeout: Duration,
    /// Buffer size for reading data
    pub buffer_size: usize,
    /// Progress reporting interval
    pub report_interval: Duration,
    /// Maximum bytes per connection before auto-disconnect
    pub max_bytes_per_connection: Option<u64>,
}

impl Default for TcpServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 1000,
            connection_timeout: Duration::from_secs(300), // 5 minutes
            read_timeout: Duration::from_secs(30),
            buffer_size: 131072, // 128KB buffer for better high-speed performance
            report_interval: Duration::from_secs(5),
            max_bytes_per_connection: Some(1_000_000_000_000), // 1TB limit for high-speed tests
        }
    }
}

/// Server metrics for monitoring
#[derive(Debug, Default)]
pub struct TcpServerMetrics {
    pub total_connections: AtomicU64,
    pub active_connections: AtomicUsize,
    pub total_bytes_received: AtomicU64,
    pub total_bytes_sent: AtomicU64,
    pub connection_errors: AtomicU64,
}

impl TcpServerMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Log current metrics
    pub fn log_summary(&self) {
        let total_conns = self.total_connections.load(Ordering::Relaxed);
        let active_conns = self.active_connections.load(Ordering::Relaxed);
        let total_bytes_received = self.total_bytes_received.load(Ordering::Relaxed);
        let total_bytes_sent = self.total_bytes_sent.load(Ordering::Relaxed);
        let errors = self.connection_errors.load(Ordering::Relaxed);

        info!(
            "Server metrics - Total connections: {}, Active: {}, Bytes received: {}, Bytes sent: {}, Errors: {}",
            total_conns,
            active_conns,
            format_bytes(total_bytes_received),
            format_bytes(total_bytes_sent),
            errors
        );
    }
}

/// Production TCP server with proper resource management and monitoring
pub struct TcpServer {
    config: TcpServerConfig,
    active_connections: Arc<AtomicUsize>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: broadcast::Sender<()>,
    metrics: Arc<TcpServerMetrics>,
}

impl TcpServer {
    pub fn new(config: TcpServerConfig) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            connection_semaphore: Arc::new(Semaphore::new(config.max_connections)),
            config,
            active_connections: Arc::new(AtomicUsize::new(0)),
            shutdown_tx,
            metrics: TcpServerMetrics::new(),
        }
    }

    pub fn get_shutdown_receiver(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    pub fn get_metrics(&self) -> Arc<TcpServerMetrics> {
        self.metrics.clone()
    }

    pub async fn shutdown(&self) -> Result<()> {
        info!("Initiating TCP server shutdown...");

        // Log final metrics before shutdown
        self.metrics.log_summary();

        let _ = self.shutdown_tx.send(());

        // Wait for connections to close gracefully
        let mut attempts = 0;
        while self.active_connections.load(Ordering::Relaxed) > 0 && attempts < 30 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            attempts += 1;
        }

        let remaining = self.active_connections.load(Ordering::Relaxed);
        if remaining > 0 {
            warn!("Force closing {} remaining connections", remaining);
        } else {
            info!("All connections closed gracefully");
        }

        // Log final metrics after shutdown
        self.metrics.log_summary();

        Ok(())
    }

    #[instrument(skip(self, addr), fields(addr = ?addr))]
    pub async fn run(&self, addr: impl ToSocketAddrs + std::fmt::Debug + Clone) -> Result<()> {
        let listener = TcpListener::bind(&addr)
            .await
            .wrap_err("Failed to bind TCP listener")?;

        let local_addr = listener
            .local_addr()
            .wrap_err("Failed to get local address")?;

        info!("TCP server listening on {}", local_addr.to_string().green());

        let connection_id = Arc::new(AtomicU64::new(0));
        let mut shutdown_rx = self.get_shutdown_receiver();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((socket, peer_addr)) => {
                            // Check if we can accept more connections
                            if let Ok(permit) = self.connection_semaphore.clone().try_acquire_owned() {
                                let conn_id = connection_id.fetch_add(1, Ordering::Relaxed);

                                info!("New TCP connection {} from {}", conn_id, peer_addr.to_string().cyan());

                                // Update metrics
                                self.metrics.total_connections.fetch_add(1, Ordering::Relaxed);
                                self.metrics.active_connections.store(
                                    self.active_connections.fetch_add(1, Ordering::Relaxed) + 1,
                                    Ordering::Relaxed
                                );

                                // Spawn connection handler
                                let handler = ProductionTcpHandler::new(
                                    conn_id,
                                    socket,
                                    peer_addr,
                                    self.config.clone(),
                                    self.active_connections.clone(),
                                    permit,
                                    self.get_shutdown_receiver(),
                                    self.metrics.clone(),
                                );

                                tokio::spawn(async move {
                                    if let Err(e) = handler.handle().await {
                                        error!("Connection {} error: {}", conn_id, e);
                                    }
                                });
                            } else {
                                warn!("Connection limit reached, rejecting connection from {}", peer_addr);
                                // Socket is dropped, connection is rejected
                            }
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                            // Brief pause to prevent tight error loops
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal, stopping accept loop");
                    break;
                }
            }
        }

        Ok(())
    }
}

// TODO: Remove this vv
/// Legacy function for backward compatibility - now uses the builder pattern
pub async fn run_tcp_server(addr: impl ToSocketAddrs + std::fmt::Debug + Clone) -> Result<()> {
    // Use the builder pattern with optimized settings for high-throughput testing
    let server = TcpServerBuilder::new()
        .max_connections(1000)
        .connection_timeout(Duration::from_secs(300))
        .read_timeout(Duration::from_secs(30))
        .buffer_size(131072) // 128KB
        .report_interval(Duration::from_secs(5))
        .max_bytes_per_connection(Some(1_000_000_000_000)) // 1TB
        .build();

    server.run(addr).await
}

/// Builder for TcpServer with sensible defaults
pub struct TcpServerBuilder {
    config: TcpServerConfig,
}

impl TcpServerBuilder {
    pub fn new() -> Self {
        Self {
            config: TcpServerConfig::default(),
        }
    }

    pub fn max_connections(mut self, max: usize) -> Self {
        self.config.max_connections = max;
        self
    }

    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.config.connection_timeout = timeout;
        self
    }

    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.config.read_timeout = timeout;
        self
    }

    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    pub fn report_interval(mut self, interval: Duration) -> Self {
        self.config.report_interval = interval;
        self
    }

    pub fn max_bytes_per_connection(mut self, max_bytes: Option<u64>) -> Self {
        self.config.max_bytes_per_connection = max_bytes;
        self
    }

    pub fn build(self) -> TcpServer {
        TcpServer::new(self.config)
    }
}

impl Default for TcpServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Production-grade TCP connection handler with comprehensive monitoring and safety features
struct ProductionTcpHandler {
    connection_id: u64,
    socket: TcpStream,
    peer_addr: std::net::SocketAddr,
    config: TcpServerConfig,
    active_connections: Arc<AtomicUsize>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    shutdown_rx: broadcast::Receiver<()>,
    stats: ConnectionStats,
    metrics: Arc<TcpServerMetrics>,
}

#[derive(Debug)]
struct ConnectionStats {
    total_bytes: AtomicU64,
    start_time: Instant,
    last_report: std::sync::Mutex<Instant>,
    last_activity: std::sync::Mutex<Instant>,
}

impl ConnectionStats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            total_bytes: AtomicU64::new(0),
            start_time: now,
            last_report: std::sync::Mutex::new(now),
            last_activity: std::sync::Mutex::new(now),
        }
    }

    fn add_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    fn should_report(&self, report_interval: Duration) -> bool {
        let mut last_report = self.last_report.lock().unwrap();
        if last_report.elapsed() >= report_interval {
            *last_report = Instant::now();
            true
        } else {
            false
        }
    }

    fn is_idle(&self, timeout: Duration) -> bool {
        self.last_activity.lock().unwrap().elapsed() > timeout
    }

    fn get_summary(&self) -> (u64, Duration, f64) {
        let bytes = self.total_bytes.load(Ordering::Relaxed);
        let duration = self.start_time.elapsed();
        let throughput_mbps = if duration.as_secs_f64() > 0.0 {
            (bytes as f64 * 8.0) / (duration.as_secs_f64() * 1_000_000.0)
        } else {
            0.0
        };
        (bytes, duration, throughput_mbps)
    }
}

impl ProductionTcpHandler {
    fn new(
        connection_id: u64,
        socket: TcpStream,
        peer_addr: std::net::SocketAddr,
        config: TcpServerConfig,
        active_connections: Arc<AtomicUsize>,
        permit: tokio::sync::OwnedSemaphorePermit,
        shutdown_rx: broadcast::Receiver<()>,
        metrics: Arc<TcpServerMetrics>,
    ) -> Self {
        Self {
            connection_id,
            socket,
            peer_addr,
            config,
            active_connections,
            _permit: permit,
            shutdown_rx,
            stats: ConnectionStats::new(),
            metrics,
        }
    }

    #[instrument(skip(self), fields(conn_id = self.connection_id, peer = %self.peer_addr))]
    async fn handle(mut self) -> Result<()> {
        debug!("Starting connection handler");

        // Set socket options for better performance
        if let Err(e) = self.configure_socket().await {
            warn!("Failed to configure socket options: {}", e);
        }

        let mut buffer = vec![0u8; self.config.buffer_size];
        let mut shutdown_rx = self.shutdown_rx.resubscribe();

        // First, read the command byte to determine if this is upload or download
        let command = match timeout(
            Duration::from_secs(5),
            self.socket.read_exact(&mut buffer[..1]),
        )
        .await
        {
            Ok(Ok(_)) => buffer[0],
            Ok(Err(e)) => {
                error!("Failed to read command byte: {}", e);
                self.metrics
                    .connection_errors
                    .fetch_add(1, Ordering::Relaxed);
                return Err(e.into());
            }
            Err(_) => {
                warn!("Timeout waiting for command byte");
                self.metrics
                    .connection_errors
                    .fetch_add(1, Ordering::Relaxed);
                return Err(eyre::eyre!("Command timeout"));
            }
        };

        let result = match command {
            b'U' => {
                let res = self.handle_upload(&mut buffer, &mut shutdown_rx).await;
                // Log final statistics for upload
                let (total_bytes, duration, throughput_mbps) = self.stats.get_summary();
                let status = if res.is_ok() { "completed" } else { "failed" };
                info!(
                    "Upload connection {} {}: {} received in {:.2}s ({})",
                    self.connection_id,
                    status,
                    format_bytes(total_bytes).yellow(),
                    duration.as_secs_f64(),
                    format_throughput(throughput_mbps).green()
                );
                res
            }
            b'D' => {
                let res = self.handle_download(&mut buffer, &mut shutdown_rx).await;
                // Log final statistics for download
                let (total_bytes, duration, throughput_mbps) = self.stats.get_summary();
                let status = if res.is_ok() { "completed" } else { "failed" };
                info!(
                    "Download connection {} {}: {} sent in {:.2}s ({})",
                    self.connection_id,
                    status,
                    format_bytes(total_bytes).yellow(),
                    duration.as_secs_f64(),
                    format_throughput(throughput_mbps).green()
                );
                res
            }
            _ => {
                warn!("Unknown command byte: {}", command);
                self.metrics
                    .connection_errors
                    .fetch_add(1, Ordering::Relaxed);
                Err(eyre::eyre!("Unknown command"))
            }
        };

        // Update active connection count and metrics
        let remaining = self.active_connections.fetch_sub(1, Ordering::Relaxed) - 1;
        self.metrics
            .active_connections
            .store(remaining, Ordering::Relaxed);
        debug!("Active connections: {}", remaining);

        result
    }

    async fn configure_socket(&mut self) -> Result<()> {
        // Configure socket for high-throughput scenarios

        // Set TCP_NODELAY to reduce latency
        if let Err(e) = self.socket.set_nodelay(true) {
            warn!("Failed to set TCP_NODELAY: {}", e);
        }

        debug!("Socket configured for high-throughput operation");
        Ok(())
    }

    async fn handle_upload(
        &mut self,
        buffer: &mut [u8],
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> Result<()> {
        info!("Handling upload request");

        loop {
            tokio::select! {
                // Handle incoming data with timeout for read operations
                read_result = timeout(self.config.read_timeout, self.socket.read(buffer)) => {
                    match read_result {
                        Ok(Ok(0)) => {
                            // Connection closed by client
                            info!("Client closed connection");
                            break Ok(());
                        }
                        Ok(Ok(n)) => {
                            self.stats.add_bytes(n as u64);

                            // Update server metrics
                            self.metrics.total_bytes_received.fetch_add(n as u64, Ordering::Relaxed);

                            // For throughput testing, we just need to consume the data
                            // as fast as possible to avoid buffer overflow

                            // Check byte limit
                            if let Some(max_bytes) = self.config.max_bytes_per_connection {
                                let total_bytes = self.stats.total_bytes.load(Ordering::Relaxed);
                                if total_bytes >= max_bytes {
                                    warn!("Connection reached byte limit ({}), closing", format_bytes(max_bytes));
                                    break Ok(());
                                }
                            }

                            // Report progress less frequently to reduce overhead
                            if self.stats.should_report(self.config.report_interval) {
                                let (total_bytes, _, throughput_mbps) = self.stats.get_summary();
                                info!(
                                    "Upload progress: {} received, {} throughput",
                                    format_bytes(total_bytes).yellow(),
                                    format_throughput(throughput_mbps).green()
                                );
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Read error: {}", e);
                            self.metrics.connection_errors.fetch_add(1, Ordering::Relaxed);
                            break Err(e.into());
                        }
                        Err(_) => {
                            warn!("Read timeout after {:?}", self.config.read_timeout);
                            self.metrics.connection_errors.fetch_add(1, Ordering::Relaxed);
                            break Err(eyre::eyre!("Read timeout"));
                        }
                    }
                }

                // Handle shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal during upload");
                    break Ok(());
                }

                // Check for idle timeout periodically
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    if self.stats.is_idle(self.config.connection_timeout) {
                        warn!("Connection idle timeout during upload");
                        break Err(eyre::eyre!("Connection idle timeout"));
                    }
                }
            }
        }
    }

    async fn handle_download(
        &mut self,
        buffer: &mut [u8],
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        info!("Handling download request");

        // Fill buffer with random data for download
        buffer.fill(0x42); // Fill with a pattern for testing

        let mut total_sent = 0u64;
        let start_time = Instant::now();
        let mut last_report = start_time;

        loop {
            tokio::select! {
                // Send data to client
                write_result = self.socket.write_all(buffer) => {
                    match write_result {
                        Ok(_) => {
                            let bytes_sent = buffer.len() as u64;
                            total_sent += bytes_sent;
                            self.stats.add_bytes(bytes_sent);

                            // Update server metrics
                            self.metrics.total_bytes_sent.fetch_add(bytes_sent, Ordering::Relaxed);

                            // Check byte limit
                            if let Some(max_bytes) = self.config.max_bytes_per_connection {
                                if total_sent >= max_bytes {
                                    info!("Connection reached byte limit ({}), closing", format_bytes(max_bytes));
                                    break Ok(());
                                }
                            }

                            // Report progress less frequently to reduce overhead
                            if last_report.elapsed() >= self.config.report_interval {
                                let elapsed = start_time.elapsed().as_secs_f64();
                                let throughput_mbps = (total_sent as f64 * 8.0) / (elapsed * 1_000_000.0);
                                info!(
                                    "Download progress: {} sent, {} throughput",
                                    format_bytes(total_sent).yellow(),
                                    format_throughput(throughput_mbps).green()
                                );
                                last_report = Instant::now();
                            }

                            // Yield control periodically to avoid overwhelming the connection
                            // This small delay helps with flow control and prevents buffer overflow
                            tokio::task::yield_now().await;
                        }
                        Err(e) => {
                            error!("Write error during download: {}", e);
                            self.metrics.connection_errors.fetch_add(1, Ordering::Relaxed);
                            break Err(e.into());
                        }
                    }
                }

                // Handle shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal during download");
                    break Ok(());
                }

                // Check for idle timeout periodically (though this is less relevant for downloads)
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    // For downloads, we don't want to timeout as quickly since we're actively sending
                    // This is just a safety check
                    debug!("Download progress check - {} sent so far", format_bytes(total_sent));
                }
            }
        }
    }
}

/// Example usage for production deployment
///
/// ```rust,no_run
/// use speed_cli::performance::tcp::server::{TcpServerBuilder, TcpServerMetrics};
/// use std::time::Duration;
/// use tracing::info;
///
/// #[tokio::main]
/// async fn main() -> eyre::Result<()> {
///     // Initialize tracing
///     tracing_subscriber::fmt::init();
///
///     // Create production server
///     let server = TcpServerBuilder::new()
///         .max_connections(1000)
///         .connection_timeout(Duration::from_secs(300))
///         .read_timeout(Duration::from_secs(30))
///         .buffer_size(65536)
///         .report_interval(Duration::from_secs(10))
///         .max_bytes_per_connection(Some(10_000_000_000)) // 10GB limit
///         .build();
///
///     // Setup graceful shutdown
///     let server_handle = {
///         let server = server.clone();
///         tokio::spawn(async move {
///             if let Err(e) = server.run("0.0.0.0:8080").await {
///                 tracing::error!("Server error: {}", e);
///             }
///         })
///     };
///
///     // Setup signal handler for graceful shutdown
///     tokio::select! {
///         _ = tokio::signal::ctrl_c() => {
///             info!("Received Ctrl+C, initiating shutdown...");
///             server.shutdown().await?;
///             server_handle.abort();
///         }
///         result = server_handle => {
///             if let Err(e) = result {
///                 tracing::error!("Server task error: {}", e);
///             }
///         }
///     }
///
///     Ok(())
/// }
/// ```
///
/// This demonstrates the complete production setup with graceful shutdown.
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_server_builder() {
        let server = TcpServerBuilder::new()
            .max_connections(100)
            .connection_timeout(Duration::from_secs(60))
            .buffer_size(32768)
            .build();

        assert_eq!(server.config.max_connections, 100);
        assert_eq!(server.config.connection_timeout, Duration::from_secs(60));
        assert_eq!(server.config.buffer_size, 32768);
    }

    #[tokio::test]
    async fn test_server_metrics() {
        let metrics = TcpServerMetrics::new();

        metrics.total_connections.store(42, Ordering::Relaxed);
        metrics.total_bytes_received.store(1024, Ordering::Relaxed);

        assert_eq!(metrics.total_connections.load(Ordering::Relaxed), 42);
        assert_eq!(metrics.total_bytes_received.load(Ordering::Relaxed), 1024);
    }

    #[tokio::test]
    async fn test_server_builder_and_metrics() {
        let server = TcpServerBuilder::new()
            .max_connections(50)
            .connection_timeout(Duration::from_secs(120))
            .buffer_size(32768)
            .report_interval(Duration::from_secs(2))
            .max_bytes_per_connection(Some(1_000_000))
            .build();

        // Test that we can get metrics
        let metrics = server.get_metrics();
        assert_eq!(metrics.total_connections.load(Ordering::Relaxed), 0);

        // Test configuration
        assert_eq!(server.config.max_connections, 50);
        assert_eq!(server.config.connection_timeout, Duration::from_secs(120));
        assert_eq!(server.config.buffer_size, 32768);
    }

    #[tokio::test]
    async fn test_server_shutdown() {
        let server = TcpServerBuilder::new().max_connections(10).build();

        // Test shutdown functionality
        let result = server.shutdown().await;
        assert!(result.is_ok());
    }
}
