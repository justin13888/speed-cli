use bytes::Bytes;
use colored::*;
use eyre::Result;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, body::Incoming as IncomingBody};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use std::sync::LazyLock;

use crate::utils::format::format_bytes;

// TODO: Clean up this code vv
// TODO: Make it possible to run HTTP and TCP server side-by-side

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    /// Bind address
    pub bind_addr: SocketAddr,
    /// Enable cors. Usually should be true.
    pub enable_cors: bool,
    /// Max upload size in bytes
    pub max_upload_size: usize,
}

pub async fn run_http_server(config: HttpServerConfig) -> Result<()> {
    let listener = TcpListener::bind(&config.bind_addr).await?;

    println!(
        "{}",
        format!("HTTP speed test server listening on {}", &config.bind_addr)
            .green()
            .bold()
    );
    println!("{}", "Available endpoints:".cyan());
    println!("  • GET  /download?size=<bytes>  - Download test data");
    println!("  • POST /upload                 - Upload test endpoint");
    println!("  • GET  /latency                - Latency test (minimal response)");
    println!("  • GET  /info                   - Server information");
    println!("  • GET  /health                 - Health check");

    let server_config = Arc::new(config);

    loop {
        let (stream, addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let config = server_config.clone();

        println!("New HTTP connection from {}", addr.to_string().cyan());

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| handle_request(req, config.clone())),
                )
                .await
            {
                eprintln!("Error serving connection from {addr}: {err:?}");
            }
        });
    }
}

async fn handle_request(
    req: Request<IncomingBody>,
    config: Arc<HttpServerConfig>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method();
    let path = req.uri().path();
    let query = req.uri().query().unwrap_or("");

    println!(
        "{} {} {}",
        method.as_str().yellow(),
        path.cyan(),
        if !query.is_empty() {
            format!("?{query}")
        } else {
            String::new()
        }
    );

    let response = match (method.as_str(), path) {
        ("GET", "/download") => handle_download(req, config.clone()).await,
        ("POST", "/upload") => handle_upload(req, config.clone()).await,
        ("GET" | "HEAD", "/latency") => handle_latency(req, config.clone()).await,
        ("GET", "/info") => handle_server_info(req, config.clone()).await,
        ("GET", "/health") => handle_health_check(req, config.clone()).await,
        ("OPTIONS", _) if config.enable_cors => handle_cors_preflight(req, config.clone()).await,
        _ => handle_not_found(req, config.clone()).await,
    };

    Ok(add_cors_headers(response, &config))
}

async fn handle_download(
    req: Request<IncomingBody>,
    _config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    let query = req.uri().query().unwrap_or("");
    let params = parse_query_params(query);

    let size = params
        .get("size")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1024 * 1024); // Default 1MB

    // Clamp size to reasonable limits (1KB to 1GB)
    let size = size.clamp(1024, 1024 * 1024 * 1024);

    // Generate random data or use a pattern
    let use_random = params.get("random").is_some_and(|v| v == "true");

    let data = if use_random {
        if size <= RANDOM_BUFFER_1MB.len() {
            // Use pre-computed random buffer
            RANDOM_BUFFER_1MB[..size].to_vec()
        } else {
            // Generate larger random data by repeating the 1MB buffer
            let mut data = Vec::with_capacity(size);
            let full_chunks = size / RANDOM_BUFFER_1MB.len();
            let remainder = size % RANDOM_BUFFER_1MB.len();

            for _ in 0..full_chunks {
                data.extend_from_slice(&RANDOM_BUFFER_1MB);
            }
            if remainder > 0 {
                data.extend_from_slice(&RANDOM_BUFFER_1MB[..remainder]);
            }
            data
        }
    } else if size <= PATTERN_BUFFER_1MB.len() {
        // Use pre-computed pattern buffer
        PATTERN_BUFFER_1MB[..size].to_vec()
    } else {
        // Generate larger pattern data by repeating the 1MB buffer
        let mut data = Vec::with_capacity(size);
        let full_chunks = size / PATTERN_BUFFER_1MB.len();
        let remainder = size % PATTERN_BUFFER_1MB.len();

        for _ in 0..full_chunks {
            data.extend_from_slice(&PATTERN_BUFFER_1MB);
        }
        if remainder > 0 {
            data.extend_from_slice(&PATTERN_BUFFER_1MB[..remainder]);
        }
        data
    };

    println!("Sending {} of test data", format_bytes(size).yellow());

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", size.to_string())
        .header("Cache-Control", "no-cache, no-store, must-revalidate")
        .body(Full::new(Bytes::from(data)))
        .unwrap()
}

async fn handle_upload(
    req: Request<IncomingBody>,
    config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    let body = req.into_body();

    match body.collect().await {
        Ok(collected) => {
            let bytes = collected.to_bytes();
            let size = bytes.len();

            if size > config.max_upload_size {
                return Response::builder()
                    .status(StatusCode::PAYLOAD_TOO_LARGE)
                    .body(Full::new(Bytes::from(format!(
                        "Upload too large: {size} bytes"
                    ))))
                    .unwrap();
            }

            println!("Received {} of upload data", format_bytes(size).yellow());

            // Return success response with upload statistics
            let response_data = serde_json::json!({
                "status": "success",
                "bytes_received": size,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(response_data.to_string())))
                .unwrap()
        }
        Err(e) => {
            eprintln!("Upload error: {e}");
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(format!("Upload failed: {e}"))))
                .unwrap()
        }
    }
}

async fn handle_latency(
    _req: Request<IncomingBody>,
    _config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    // Minimal response for latency testing
    let response_data = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "server": "speed-cli-http-server"
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header(
            "Content-Length",
            response_data.to_string().len().to_string(),
        )
        .body(Full::new(Bytes::from(response_data.to_string())))
        .unwrap()
}

async fn handle_server_info(
    _req: Request<IncomingBody>,
    config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    let info = serde_json::json!({
        "server": "speed-cli-http-server",
        "version": env!("CARGO_PKG_VERSION"),
        "endpoints": {
            "download": "/download?size=<bytes>&random=<true|false>",
            "upload": "/upload",
            "latency": "/latency",
            "health": "/health"
        },
        "config": {
            "max_upload_size": config.max_upload_size,
            "cors_enabled": config.enable_cors
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(info.to_string())))
        .unwrap()
}

async fn handle_health_check(
    _req: Request<IncomingBody>,
    _config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    let health = serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "uptime": "running" // Could track actual uptime
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(health.to_string())))
        .unwrap()
}

async fn handle_cors_preflight(
    _req: Request<IncomingBody>,
    _config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization",
        )
        .header("Access-Control-Max-Age", "86400")
        .body(Full::new(Bytes::new()))
        .unwrap()
}

async fn handle_not_found(
    _req: Request<IncomingBody>,
    _config: Arc<HttpServerConfig>,
) -> Response<Full<Bytes>> {
    let error = serde_json::json!({
        "error": "Not Found",
        "message": "The requested endpoint does not exist",
        "available_endpoints": ["/download", "/upload", "/latency", "/info", "/health"]
    });

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(error.to_string())))
        .unwrap()
}

fn add_cors_headers(
    mut response: Response<Full<Bytes>>,
    config: &HttpServerConfig,
) -> Response<Full<Bytes>> {
    if config.enable_cors {
        let headers = response.headers_mut();
        headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
        headers.insert("Access-Control-Allow-Credentials", "true".parse().unwrap());
        headers.insert(
            "Access-Control-Allow-Headers",
            "Origin, X-Requested-With, Content-Type, Accept, Authorization"
                .parse()
                .unwrap(),
        );
    }
    response
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    for pair in query.split('&') {
        let mut iter = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (iter.next(), iter.next()) {
            params.insert(
                urlencoding::decode(key).unwrap_or_default().to_string(),
                urlencoding::decode(value).unwrap_or_default().to_string(),
            );
        }
    }

    params
}

// Pre-generated test data buffers to avoid repeated generation
static PATTERN_BUFFER_1MB: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let pattern = b"SpeedTestData0123456789ABCDEF";
    let mut data = Vec::with_capacity(1024 * 1024);
    for i in 0..(1024 * 1024) {
        data.push(pattern[i % pattern.len()]);
    }
    data
});

static RANDOM_BUFFER_1MB: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let mut data = vec![0u8; 1024 * 1024];
    use rand::RngCore;
    rand::rng().fill_bytes(&mut data);
    data
});

// Additional utility functions for HTTP/2 server (future enhancement)
pub async fn run_http2_server(config: HttpServerConfig) -> Result<()> {
    // HTTP/2 server implementation would go here
    // This would require additional TLS setup and HTTP/2 specific handling
    println!("HTTP/2 server support - coming soon!");
    Ok(())
}
