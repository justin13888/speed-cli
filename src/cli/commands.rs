use crate::speed::http::{HttpTestType, HttpVersion};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
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

    /// Run HTTP speed test (similar to Ookla)
    Http {
        /// Server URL (e.g., http://localhost:8080)
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,

        /// Test duration in seconds
        #[arg(short, long, default_value = "10")]
        time: u64,

        /// Number of parallel connections
        #[arg(short, long, default_value = "4")]
        parallel: usize,

        /// HTTP version (http1, http2, auto)
        #[arg(long, default_value = "auto")]
        version: HttpVersion,

        /// Test type (download, upload, bidirectional, latency, comprehensive)
        #[arg(long = "type", default_value = "comprehensive")]
        test_type: HttpTestType,

        /// Export results to file (json or csv)
        #[arg(short, long)]
        export: Option<String>,

        /// Enable adaptive test sizing
        #[arg(long)]
        adaptive: bool,
    },

    /// Run HTTP speed test server
    HttpServer {
        /// Listen port
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Bind to specific interface
        #[arg(short, long, default_value = "0.0.0.0")]
        bind: String,

        /// Maximum upload size in MB
        #[arg(long, default_value = "100")]
        max_upload_mb: usize,

        /// Enable CORS headers
        #[arg(long, default_value = "true")]
        cors: bool,
    },

    /// Run comprehensive network diagnostics
    Diagnostics {
        /// Server URL for testing
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,

        /// Test duration in seconds
        #[arg(short, long, default_value = "30")]
        time: u64,

        /// Number of parallel connections for HTTP tests
        #[arg(short, long, default_value = "4")]
        parallel: usize,

        /// Export results to file (json only)
        #[arg(short, long)]
        export: Option<String>,

        /// Skip DNS performance tests
        #[arg(long)]
        skip_dns: bool,

        /// Skip connection quality tests
        #[arg(long)]
        skip_quality: bool,

        /// Skip network topology analysis
        #[arg(long)]
        skip_topology: bool,
    },
}
