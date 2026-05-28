use colored::*;
use eyre::{Context, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, broadcast};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::utils::format::{format_bytes, format_throughput};

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
    /// Maximum concurrent connections from a single source IP. `None`
    /// disables the per-IP cap entirely.
    pub max_connections_per_ip: Option<usize>,
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
            max_connections_per_ip: Some(32),
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

/// Per-source-IP connection counts. Decremented automatically by
/// `PerIpGuard` on drop so we don't leak counts when handlers panic.
type PerIpMap = Arc<Mutex<HashMap<IpAddr, usize>>>;

/// RAII guard for per-IP connection counts.
struct PerIpGuard {
    map: PerIpMap,
    ip: IpAddr,
}

impl Drop for PerIpGuard {
    fn drop(&mut self) {
        let mut m = self.map.lock();
        match m.get_mut(&self.ip) {
            Some(count) if *count > 1 => {
                *count -= 1;
            }
            Some(_) => {
                m.remove(&self.ip);
            }
            None => {}
        }
    }
}

/// Production TCP server with proper resource management and monitoring
pub struct TcpServer {
    config: TcpServerConfig,
    active_connections: Arc<AtomicUsize>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: broadcast::Sender<()>,
    metrics: Arc<TcpServerMetrics>,
    per_ip: PerIpMap,
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
            per_ip: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Try to admit one connection from `ip`. Returns Some(guard) on
    /// success, None when the per-IP cap is exceeded.
    fn try_admit_ip(&self, ip: IpAddr) -> Option<PerIpGuard> {
        let cap = self.config.max_connections_per_ip;
        let mut m = self.per_ip.lock();
        let count = m.entry(ip).or_insert(0);
        if let Some(limit) = cap
            && *count >= limit
        {
            return None;
        }
        *count += 1;
        Some(PerIpGuard {
            map: self.per_ip.clone(),
            ip,
        })
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
        self.run_on(listener).await
    }

    /// Run the accept loop on a listener that is already bound. Used by
    /// the ephemeral-port server startup path, which binds every
    /// listener up front so the control manifest can advertise the real
    /// ports.
    pub async fn run_on(&self, listener: TcpListener) -> Result<()> {
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
                            // Check the per-IP cap first so a single noisy
                            // peer can't burn all of `max_connections`.
                            let Some(ip_guard) = self.try_admit_ip(peer_addr.ip()) else {
                                warn!(
                                    "Per-IP connection cap reached for {}, rejecting connection",
                                    peer_addr.ip()
                                );
                                continue;
                            };

                            // Then the global concurrent-connections cap.
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
                                    TcpHandlerContext {
                                        active_connections: self.active_connections.clone(),
                                        permit,
                                        shutdown_rx: self.get_shutdown_receiver(),
                                        metrics: self.metrics.clone(),
                                        ip_guard,
                                    },
                                );

                                tokio::spawn(async move {
                                    if let Err(e) = handler.handle().await {
                                        error!("Connection {} error: {}", conn_id, e);
                                    }
                                });
                            } else {
                                warn!("Connection limit reached, rejecting connection from {}", peer_addr);
                                // ip_guard drops here, releasing the per-IP slot
                                drop(ip_guard);
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

/// Run a TCP server with default high-throughput settings, gracefully
/// shutting down when `cancel` fires.
pub async fn run_tcp_server(
    addr: impl ToSocketAddrs + std::fmt::Debug + Clone,
    cancel: CancellationToken,
) -> Result<()> {
    let server = Arc::new(
        TcpServerBuilder::new()
            .max_connections(1000)
            .connection_timeout(Duration::from_secs(300))
            .read_timeout(Duration::from_secs(30))
            .buffer_size(131072) // 128KB
            .report_interval(Duration::from_secs(5))
            .max_bytes_per_connection(Some(1_000_000_000_000)) // 1TB
            .build(),
    );

    let server_for_shutdown = server.clone();
    let shutdown_task = tokio::spawn(async move {
        cancel.cancelled().await;
        if let Err(e) = server_for_shutdown.shutdown().await {
            error!("TCP server shutdown error: {}", e);
        }
    });

    let result = server.run(addr).await;
    shutdown_task.abort();
    result
}

/// Like [`run_tcp_server`] but drives a listener that is already bound
/// (the ephemeral-port startup path).
pub async fn run_tcp_server_on(listener: TcpListener, cancel: CancellationToken) -> Result<()> {
    let server = Arc::new(TcpServerBuilder::new().build());
    let server_for_shutdown = server.clone();
    let shutdown_task = tokio::spawn(async move {
        cancel.cancelled().await;
        if let Err(e) = server_for_shutdown.shutdown().await {
            error!("TCP server shutdown error: {}", e);
        }
    });
    let result = server.run_on(listener).await;
    shutdown_task.abort();
    result
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

    pub fn max_connections_per_ip(mut self, max: Option<usize>) -> Self {
        self.config.max_connections_per_ip = max;
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

/// Context for creating a new TCP connection handler
struct TcpHandlerContext {
    pub active_connections: Arc<AtomicUsize>,
    pub permit: tokio::sync::OwnedSemaphorePermit,
    pub shutdown_rx: broadcast::Receiver<()>,
    pub metrics: Arc<TcpServerMetrics>,
    pub ip_guard: PerIpGuard,
}

/// Production-grade TCP connection handler with comprehensive monitoring and safety features
struct ProductionTcpHandler {
    connection_id: u64,
    socket: TcpStream,
    peer_addr: std::net::SocketAddr,
    config: TcpServerConfig,
    active_connections: Arc<AtomicUsize>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    _ip_guard: PerIpGuard,
    shutdown_rx: broadcast::Receiver<()>,
    stats: ConnectionStats,
    metrics: Arc<TcpServerMetrics>,
}

#[derive(Debug)]
struct ConnectionStats {
    total_bytes: AtomicU64,
    start_time: Instant,
    last_report: Mutex<Instant>,
    last_activity: Mutex<Instant>,
}

impl ConnectionStats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            total_bytes: AtomicU64::new(0),
            start_time: now,
            last_report: Mutex::new(now),
            last_activity: Mutex::new(now),
        }
    }

    fn add_bytes(&self, bytes: u64) {
        self.total_bytes.fetch_add(bytes, Ordering::Relaxed);
        *self.last_activity.lock() = Instant::now();
    }

    fn should_report(&self, report_interval: Duration) -> bool {
        let mut last_report = self.last_report.lock();
        if last_report.elapsed() >= report_interval {
            *last_report = Instant::now();
            true
        } else {
            false
        }
    }

    fn is_idle(&self, timeout: Duration) -> bool {
        self.last_activity.lock().elapsed() > timeout
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
        context: TcpHandlerContext,
    ) -> Self {
        Self {
            connection_id,
            socket,
            peer_addr,
            config,
            active_connections: context.active_connections,
            _permit: context.permit,
            _ip_guard: context.ip_guard,
            shutdown_rx: context.shutdown_rx,
            stats: ConnectionStats::new(),
            metrics: context.metrics,
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
            b'F' => {
                let res = self.handle_full_duplex(&mut shutdown_rx).await;
                let (total_bytes, duration, throughput_mbps) = self.stats.get_summary();
                let status = if res.is_ok() { "completed" } else { "failed" };
                info!(
                    "Full-duplex connection {} {}: {} read in {:.2}s ({} aggregate)",
                    self.connection_id,
                    status,
                    format_bytes(total_bytes).yellow(),
                    duration.as_secs_f64(),
                    format_throughput(throughput_mbps).green()
                );
                res
            }
            b'P' => {
                let res = self.handle_ping(&mut shutdown_rx).await;
                info!(
                    "Ping connection {} {}",
                    self.connection_id,
                    if res.is_ok() { "completed" } else { "failed" }
                );
                res
            }
            b'H' => {
                let res = crate::performance::tcp::handshake::server_hello(
                    &mut self.socket,
                    self.peer_addr,
                )
                .await;
                if let Err(e) = &res {
                    debug!("Hello connection {} failed: {}", self.connection_id, e);
                }
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
        use rand::RngCore as _;
        use tokio::io::AsyncWriteExt;

        info!("Handling download request");

        // Fill buffer with random data so any path-compression middleboxes
        // (some VPNs, modems) can't deflate the stream and inflate the
        // reported throughput. Done once per connection - cheap.
        rand::rng().fill_bytes(buffer);

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
                            if let Some(max_bytes) = self.config.max_bytes_per_connection
                                && total_sent >= max_bytes {
                                    info!("Connection reached byte limit ({}), closing", format_bytes(max_bytes));
                                    break Ok(());
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

    /// Handle a full-duplex test: read and write concurrently on the same
    /// stream until either side hits its byte limit, the peer closes, or
    /// shutdown is signaled.
    async fn handle_full_duplex(
        &mut self,
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> Result<()> {
        use rand::RngCore as _;
        use tokio::io::AsyncWriteExt;

        info!("Handling full-duplex request");

        let (mut read_half, mut write_half) = self.socket.split();
        let mut read_buf = vec![0u8; self.config.buffer_size];
        let mut write_buf = vec![0u8; self.config.buffer_size];
        rand::rng().fill_bytes(&mut write_buf);

        let mut total_sent: u64 = 0;
        let read_timeout = self.config.read_timeout;
        let max_bytes = self.config.max_bytes_per_connection;

        loop {
            tokio::select! {
                read_result = timeout(read_timeout, read_half.read(&mut read_buf)) => {
                    match read_result {
                        Ok(Ok(0)) => {
                            info!("Full-duplex: peer closed read half");
                            break Ok(());
                        }
                        Ok(Ok(n)) => {
                            self.stats.add_bytes(n as u64);
                            self.metrics.total_bytes_received.fetch_add(n as u64, Ordering::Relaxed);
                        }
                        Ok(Err(e)) => {
                            error!("Full-duplex read error: {}", e);
                            self.metrics.connection_errors.fetch_add(1, Ordering::Relaxed);
                            break Err(e.into());
                        }
                        Err(_) => {
                            warn!("Full-duplex read timeout after {:?}", read_timeout);
                            break Err(eyre::eyre!("Read timeout"));
                        }
                    }
                }
                write_result = write_half.write_all(&write_buf) => {
                    match write_result {
                        Ok(()) => {
                            let n = write_buf.len() as u64;
                            total_sent += n;
                            self.metrics.total_bytes_sent.fetch_add(n, Ordering::Relaxed);
                            if let Some(limit) = max_bytes
                                && total_sent >= limit {
                                    info!("Full-duplex: write side hit byte limit");
                                    break Ok(());
                                }
                        }
                        Err(e) => {
                            error!("Full-duplex write error: {}", e);
                            self.metrics.connection_errors.fetch_add(1, Ordering::Relaxed);
                            break Err(e.into());
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Full-duplex: shutdown signal");
                    break Ok(());
                }
            }
        }
    }

    /// Handle an in-stream ping/pong session. The client writes 8-byte
    /// little-endian timestamps; we echo each one back as soon as it
    /// arrives. This measures application-level RTT on a *warm* TCP
    /// connection, which is what most "ping over TCP" benchmarks
    /// actually want - distinct from the connect-time latency that the
    /// older client mode measured.
    async fn handle_ping(
        &mut self,
        shutdown_rx: &mut broadcast::Receiver<()>,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        debug!("Handling ping request");
        let mut buf = [0u8; 8];
        loop {
            tokio::select! {
                read = timeout(self.config.read_timeout, self.socket.read_exact(&mut buf)) => {
                    match read {
                        Ok(Ok(_)) => {
                            self.stats.add_bytes(8);
                            if let Err(e) = self.socket.write_all(&buf).await {
                                debug!("ping echo write error: {}", e);
                                break Ok(());
                            }
                        }
                        Ok(Err(e)) => {
                            // EOF or read error - peer closed cleanly,
                            // not an error worth flagging.
                            debug!("ping read ended: {}", e);
                            break Ok(());
                        }
                        Err(_) => {
                            // Idle past read_timeout - peer probably done.
                            break Ok(());
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    break Ok(());
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
