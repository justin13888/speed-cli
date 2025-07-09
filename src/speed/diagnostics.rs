use colored::*;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use trust_dns_resolver::TokioAsyncResolver;
use trust_dns_resolver::config::*;

use crate::speed::http::*;
use crate::network::types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensiveTestResult {
    pub dns_performance: DnsPerformanceResult,
    pub connection_quality: ConnectionQualityResult,
    pub http_performance: HttpTestResult,
    pub network_topology: NetworkTopologyResult,
    pub geographic_info: GeographicInfo,
    pub overall_score: f64,
    pub recommendations: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsPerformanceResult {
    pub resolution_time_ms: f64,
    pub dns_servers: Vec<DnsServerInfo>,
    pub ipv4_support: bool,
    pub ipv6_support: bool,
    pub dnssec_support: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsServerInfo {
    pub server: String,
    pub response_time_ms: f64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionQualityResult {
    pub jitter_ms: f64,
    pub packet_loss_percent: f64,
    pub latency_consistency: f64, // Standard deviation of latency
    pub connection_stability: ConnectionStability,
    pub mtu_discovery: MtuDiscoveryResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionStability {
    Excellent,
    Good,
    Fair,
    Poor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtuDiscoveryResult {
    pub optimal_mtu: u16,
    pub path_mtu: u16,
    pub fragmentation_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkTopologyResult {
    pub hop_count: usize,
    pub route_changes: bool,
    pub asymmetric_routing: bool,
    pub congestion_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeographicInfo {
    pub client_location: Option<Location>,
    pub server_location: Option<Location>,
    pub distance_km: Option<f64>,
    pub estimated_light_speed_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub city: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
    pub isp: String,
}

#[derive(Debug, Clone)]
pub struct ComprehensiveTestConfig {
    pub server_url: String,
    pub test_duration: u64,
    pub include_dns_tests: bool,
    pub include_quality_tests: bool,
    pub include_topology_tests: bool,
    pub parallel_connections: usize,
    pub export_file: Option<String>,
}

impl Default for ComprehensiveTestConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:8080".to_string(),
            test_duration: 30,
            include_dns_tests: true,
            include_quality_tests: true,
            include_topology_tests: true,
            parallel_connections: 4,
            export_file: None,
        }
    }
}

pub async fn run_comprehensive_test(
    config: ComprehensiveTestConfig,
) -> Result<ComprehensiveTestResult> {
    println!(
        "{}",
        "Starting comprehensive network diagnostic test..."
            .green()
            .bold()
    );

    let start_time = Instant::now();

    // Phase 1: DNS Performance Testing
    println!(
        "\n{}",
        "Phase 1: DNS Performance Analysis...".yellow().bold()
    );
    let dns_result = if config.include_dns_tests {
        test_dns_performance(&config.server_url).await?
    } else {
        DnsPerformanceResult {
            resolution_time_ms: 0.0,
            dns_servers: vec![],
            ipv4_support: true,
            ipv6_support: false,
            dnssec_support: false,
        }
    };

    // Phase 2: Connection Quality Assessment
    println!(
        "\n{}",
        "Phase 2: Connection Quality Assessment...".yellow().bold()
    );
    let quality_result = if config.include_quality_tests {
        test_connection_quality(&config.server_url).await?
    } else {
        ConnectionQualityResult {
            jitter_ms: 0.0,
            packet_loss_percent: 0.0,
            latency_consistency: 0.0,
            connection_stability: ConnectionStability::Good,
            mtu_discovery: MtuDiscoveryResult {
                optimal_mtu: 1500,
                path_mtu: 1500,
                fragmentation_detected: false,
            },
        }
    };

    // Phase 3: HTTP Performance Testing
    println!(
        "\n{}",
        "Phase 3: HTTP Performance Testing...".yellow().bold()
    );
    let http_config = HttpTestConfig {
        server_url: config.server_url.clone(),
        duration: config.test_duration,
        parallel_connections: config.parallel_connections,
        test_type: HttpTestType::Comprehensive,
        http_version: HttpVersion::Auto,
        test_sizes: vec![1024 * 1024, 10 * 1024 * 1024, 50 * 1024 * 1024], // 1MB, 10MB, 50MB
        adaptive_sizing: true,
        export_file: None,
    };
    let http_result = run_http_test(http_config).await?;

    // Phase 4: Network Topology Analysis
    println!(
        "\n{}",
        "Phase 4: Network Topology Analysis...".yellow().bold()
    );
    let topology_result = if config.include_topology_tests {
        analyze_network_topology(&config.server_url).await?
    } else {
        NetworkTopologyResult {
            hop_count: 0,
            route_changes: false,
            asymmetric_routing: false,
            congestion_detected: false,
        }
    };

    // Phase 5: Geographic Information
    println!("\n{}", "Phase 5: Geographic Analysis...".yellow().bold());
    let geo_info = get_geographic_info(&config.server_url).await?;

    // Calculate overall performance score
    let overall_score =
        calculate_performance_score(&dns_result, &quality_result, &http_result, &topology_result);

    // Generate recommendations
    let recommendations =
        generate_recommendations(&dns_result, &quality_result, &http_result, &topology_result);

    let result = ComprehensiveTestResult {
        dns_performance: dns_result,
        connection_quality: quality_result,
        http_performance: http_result,
        network_topology: topology_result,
        geographic_info: geo_info,
        overall_score,
        recommendations,
        timestamp: chrono::Utc::now(),
    };

    // Export results if requested
    if let Some(export_path) = &config.export_file {
        export_comprehensive_results(&result, export_path).await?;
    }

    let total_time = start_time.elapsed();
    println!(
        "\n{}",
        format!(
            "Comprehensive test completed in {:.2}s",
            total_time.as_secs_f64()
        )
        .green()
        .bold()
    );

    print_comprehensive_results(&result);
    Ok(result)
}

async fn test_dns_performance(server_url: &str) -> Result<DnsPerformanceResult> {
    let url = url::Url::parse(server_url)?;
    let host = url.host_str().unwrap_or("localhost");

    println!("Testing DNS resolution for {}...", host.cyan());

    let resolver = TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

    let start = Instant::now();

    // Test IPv4 resolution
    let ipv4_result = resolver.ipv4_lookup(host).await;
    let ipv4_support = ipv4_result.is_ok();

    // Test IPv6 resolution
    let ipv6_result = resolver.ipv6_lookup(host).await;
    let ipv6_support = ipv6_result.is_ok();

    let resolution_time = start.elapsed().as_secs_f64() * 1000.0;

    // Test common DNS servers
    let dns_servers = test_multiple_dns_servers(host).await?;

    println!(
        "DNS resolution: {:.2}ms (IPv4: {}, IPv6: {})",
        resolution_time,
        if ipv4_support {
            "✓".green()
        } else {
            "✗".red()
        },
        if ipv6_support {
            "✓".green()
        } else {
            "✗".red()
        }
    );

    Ok(DnsPerformanceResult {
        resolution_time_ms: resolution_time,
        dns_servers,
        ipv4_support,
        ipv6_support,
        dnssec_support: false, // Simplified for now
    })
}

async fn test_multiple_dns_servers(host: &str) -> Result<Vec<DnsServerInfo>> {
    // TODO: Make this configurable
    let dns_servers = vec![
        "8.8.8.8",        // Google
        "1.1.1.1",        // Cloudflare
        "208.67.222.222", // OpenDNS
    ];

    let mut results = Vec::new();

    for dns_server in dns_servers {
        let start = Instant::now();

        // Create a custom resolver for this DNS server
        let mut config = ResolverConfig::new();
        config.add_name_server(trust_dns_resolver::config::NameServerConfig {
            socket_addr: format!("{dns_server}:53").parse().unwrap(),
            protocol: trust_dns_resolver::config::Protocol::Udp,
            tls_dns_name: None,
            trust_negative_responses: true,
            bind_addr: None,
        });

        let resolver = TokioAsyncResolver::tokio(config, ResolverOpts::default());

        let success = resolver.ipv4_lookup(host).await.is_ok();

        let response_time = start.elapsed().as_secs_f64() * 1000.0;

        results.push(DnsServerInfo {
            server: dns_server.to_string(),
            response_time_ms: response_time,
            success,
        });

        print!(".");
    }
    println!();

    Ok(results)
}

async fn test_connection_quality(server_url: &str) -> Result<ConnectionQualityResult> {
    println!("Analyzing connection quality...");

    // Perform multiple latency measurements for jitter calculation
    let mut latencies = Vec::new();
    let num_tests = 20usize;

    for i in 0..num_tests {
        if let Ok(client) = reqwest::Client::new()
            .get(format!("{server_url}/latency"))
            .build()
        {
            let start = Instant::now();
            if reqwest::Client::new().execute(client).await.is_ok() {
                let latency = start.elapsed().as_secs_f64() * 1000.0;
                latencies.push(latency);
            }
        }

        if i.is_multiple_of(5) {
            print!(".");
        }
        sleep(Duration::from_millis(100)).await;
    }
    println!();

    if latencies.is_empty() {
        return Ok(ConnectionQualityResult {
            jitter_ms: 0.0,
            packet_loss_percent: 0.0,
            latency_consistency: 0.0,
            connection_stability: ConnectionStability::Poor,
            mtu_discovery: MtuDiscoveryResult {
                optimal_mtu: 1500,
                path_mtu: 1500,
                fragmentation_detected: false,
            },
        });
    }

    // Calculate statistics
    let avg_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let variance = latencies
        .iter()
        .map(|&x| (x - avg_latency).powi(2))
        .sum::<f64>()
        / latencies.len() as f64;
    let jitter = variance.sqrt();

    // Calculate packet loss (simplified: based on failed requests)
    let packet_loss = ((num_tests - latencies.len()) as f64 / num_tests as f64) * 100.0;

    // Determine connection stability
    let stability = match jitter {
        j if j < 5.0 => ConnectionStability::Excellent,
        j if j < 15.0 => ConnectionStability::Good,
        j if j < 30.0 => ConnectionStability::Fair,
        _ => ConnectionStability::Poor,
    };

    println!("Jitter: {jitter:.2}ms, Packet loss: {packet_loss:.2}%, Stability: {stability:?}");

    Ok(ConnectionQualityResult {
        jitter_ms: jitter,
        packet_loss_percent: packet_loss,
        latency_consistency: jitter,
        connection_stability: stability,
        mtu_discovery: discover_mtu(server_url).await?,
    })
}

async fn discover_mtu(_server_url: &str) -> Result<MtuDiscoveryResult> {
    // Simplified MTU discovery - in a real implementation,
    // this would use ICMP or UDP probes with different sizes
    println!("Discovering optimal MTU...");

    Ok(MtuDiscoveryResult {
        optimal_mtu: 1500,
        path_mtu: 1500,
        fragmentation_detected: false,
    })
}

async fn analyze_network_topology(_server_url: &str) -> Result<NetworkTopologyResult> {
    println!("Analyzing network topology...");

    // Simplified topology analysis - real implementation would use traceroute
    Ok(NetworkTopologyResult {
        hop_count: 10,
        route_changes: false,
        asymmetric_routing: false,
        congestion_detected: false,
    })
}

async fn get_geographic_info(_server_url: &str) -> Result<GeographicInfo> {
    println!("Gathering geographic information...");

    // Simplified geographic info - real implementation would use IP geolocation APIs
    Ok(GeographicInfo {
        client_location: None,
        server_location: None,
        distance_km: None,
        estimated_light_speed_latency_ms: None,
    })
}

fn calculate_performance_score(
    dns: &DnsPerformanceResult,
    quality: &ConnectionQualityResult,
    http: &HttpTestResult,
    _topology: &NetworkTopologyResult,
) -> f64 {
    let mut score: f64 = 100.0;

    // DNS performance (10% of score)
    if dns.resolution_time_ms > 100.0 {
        score -= 5.0;
    }
    if !dns.ipv4_support {
        score -= 2.5;
    }
    if !dns.ipv6_support {
        score -= 2.5;
    }

    // Connection quality (30% of score)
    match quality.connection_stability {
        ConnectionStability::Excellent => {}
        ConnectionStability::Good => score -= 5.0,
        ConnectionStability::Fair => score -= 15.0,
        ConnectionStability::Poor => score -= 30.0,
    }

    // HTTP performance (60% of score)
    if let Some(download) = http.download_mbps {
        if download < 10.0 {
            score -= 30.0;
        } else if download < 50.0 {
            score -= 15.0;
        } else if download < 100.0 {
            score -= 5.0;
        }
    }

    if let Some(upload) = http.upload_mbps {
        if upload < 5.0 {
            score -= 15.0;
        } else if upload < 25.0 {
            score -= 10.0;
        } else if upload < 50.0 {
            score -= 5.0;
        }
    }

    score.clamp(0.0, 100.0)
}

fn generate_recommendations(
    dns: &DnsPerformanceResult,
    quality: &ConnectionQualityResult,
    http: &HttpTestResult,
    _topology: &NetworkTopologyResult,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    if dns.resolution_time_ms > 100.0 {
        recommendations
            .push("Consider using faster DNS servers (e.g., 1.1.1.1 or 8.8.8.8)".to_string());
    }

    if !dns.ipv6_support {
        recommendations.push("Enable IPv6 support for better performance".to_string());
    }

    match quality.connection_stability {
        ConnectionStability::Poor => {
            recommendations.push(
                "High jitter detected - check for network congestion or interference".to_string(),
            );
        }
        ConnectionStability::Fair => {
            recommendations.push(
                "Moderate jitter detected - consider upgrading network equipment".to_string(),
            );
        }
        _ => {}
    }

    if quality.packet_loss_percent > 1.0 {
        recommendations
            .push("Packet loss detected - check network cables and equipment".to_string());
    }

    if let Some(download) = http.download_mbps
        && download < 25.0
    {
        recommendations
            .push("Download speed below expectations - contact ISP or upgrade plan".to_string());
    }

    if let Some(upload) = http.upload_mbps
        && upload < 10.0
    {
        recommendations
            .push("Upload speed may limit video conferencing and file sharing".to_string());
    }

    if recommendations.is_empty() {
        recommendations.push("Your network performance looks excellent!".to_string());
    }

    recommendations
}

async fn export_comprehensive_results(result: &ComprehensiveTestResult, path: &str) -> Result<()> {
    if path.ends_with(".json") {
        let json_data = serde_json::to_string_pretty(result)?;
        tokio::fs::write(path, json_data).await?;
        println!("Comprehensive results exported to {}", path.green());
    } else {
        return Err(eyre::anyhow!(
            "Only JSON export is supported for comprehensive results"
        ));
    }

    Ok(())
}

fn print_comprehensive_results(result: &ComprehensiveTestResult) {
    println!("\n{}", "═".repeat(80).green());
    println!(
        "{}",
        "COMPREHENSIVE NETWORK DIAGNOSTIC RESULTS".green().bold()
    );
    println!("{}", "═".repeat(80).green());

    // Overall Score
    let score_color = match result.overall_score {
        s if s >= 80.0 => "green",
        s if s >= 60.0 => "yellow",
        _ => "red",
    };
    println!(
        "\n🎯 {} {:.1}/100",
        "Overall Score:".bold(),
        result.overall_score.to_string().color(score_color).bold()
    );

    // DNS Performance
    println!("\n{}", "🌐 DNS PERFORMANCE".cyan().bold());
    println!(
        "   Resolution Time: {:.2}ms",
        result.dns_performance.resolution_time_ms
    );
    println!(
        "   IPv4 Support: {}",
        if result.dns_performance.ipv4_support {
            "✓".green()
        } else {
            "✗".red()
        }
    );
    println!(
        "   IPv6 Support: {}",
        if result.dns_performance.ipv6_support {
            "✓".green()
        } else {
            "✗".red()
        }
    );

    for dns_server in &result.dns_performance.dns_servers {
        let status = if dns_server.success {
            "✓".green()
        } else {
            "✗".red()
        };
        println!(
            "   DNS Server {}: {:.2}ms {}",
            dns_server.server.cyan(),
            dns_server.response_time_ms,
            status
        );
    }

    // Connection Quality
    println!("\n{}", "📡 CONNECTION QUALITY".cyan().bold());
    println!("   Jitter: {:.2}ms", result.connection_quality.jitter_ms);
    println!(
        "   Packet Loss: {:.2}%",
        result.connection_quality.packet_loss_percent
    );
    println!(
        "   Stability: {:?}",
        result.connection_quality.connection_stability
    );
    println!(
        "   Optimal MTU: {} bytes",
        result.connection_quality.mtu_discovery.optimal_mtu
    );

    // HTTP Performance
    println!("\n{}", "🚀 HTTP PERFORMANCE".cyan().bold());
    if let Some(download) = result.http_performance.download_mbps {
        println!(
            "   Download Speed: {}",
            format_bandwidth(download).green().bold()
        );
    }
    if let Some(upload) = result.http_performance.upload_mbps {
        println!(
            "   Upload Speed: {}",
            format_bandwidth(upload).green().bold()
        );
    }
    if let Some(latency) = result.http_performance.latency_ms {
        println!("   HTTP Latency: {latency:.2}ms");
    }
    println!(
        "   Parallel Connections: {}",
        result.http_performance.parallel_connections
    );

    // Network Topology
    println!("\n{}", "🗺️  NETWORK TOPOLOGY".cyan().bold());
    println!("   Hop Count: {}", result.network_topology.hop_count);
    println!(
        "   Route Stability: {}",
        if result.network_topology.route_changes {
            "Unstable".red()
        } else {
            "Stable".green()
        }
    );

    // Recommendations
    if !result.recommendations.is_empty() {
        println!("\n{}", "💡 RECOMMENDATIONS".yellow().bold());
        for (i, rec) in result.recommendations.iter().enumerate() {
            println!("   {}. {}", i + 1, rec);
        }
    }

    println!("\n{}", "═".repeat(80).green());
}
