# speed-cli

> Disclaimer: Tool is under active development. Some features are to be improved in correctness and documentation. Open to contributions!

A comprehensive network performance measurement tool with for TCP, UDP, and HTTP protocols.

## Why Another Network Testing Tool?

It's difficult to have one tool that tests your network conditions between two devices in a way that is representative of real-world traffic. Most tools focus and excel at specific aspects (e.g. iperf3 with TCP/UDP, speed test cools for HTTP throughput/latency). However, applications use a mixture of protocols with various characteristics. This tool is a framework to create synthetic but realistic network loads between any two devices, on any platform using [Rust](https://www.rust-lang.org/).

## Features

### Core Network Testing

- **TCP/UDP Throughput Testing** (similar to iperf3)
- **HTTP/1.1 and HTTP/2 Speed Tests** (similar to Ookla)
- **Multi-connection Parallel Testing**
- **Bidirectional Testing** (simultaneous upload/download)

### Advanced Diagnostics

- **DNS Performance Analysis**
  - Resolution time measurement
  - Multiple DNS server testing
  - IPv4/IPv6 support detection
- **Connection Quality Assessment**
  - Jitter measurement
  - Packet loss detection
  - Latency consistency analysis
  - Connection stability scoring
- **Network Topology Analysis**
  - MTU discovery
  - Route stability detection
  - Congestion analysis
- **Geographic Information**
  - IP geolocation
  - Distance calculations
  - Theoretical vs actual latency comparison

### Quality Metrics

- **Bandwidth** (upload/download)
- **Latency** (round-trip time)
- **Jitter** (latency variation)
- **Packet Loss**
- **Connection Stability**
- **DNS Resolution Performance**

### Export and Reporting

- **Multiple formats:** JSON, CSV
- **Comprehensive results:** All metrics in single report
- **Historical data:** Timestamp and metadata included
- **Performance scoring:** Overall network quality assessment

## Installation

Build from source:

```bash
# Prerequisite: Rust installed via [rustup](https://rustup.rs/)
git clone https://github.com/justin13888/speed-cli
cd speed-cli
cargo install --path .
```

The binary will be available at `target/release/speed-cli` (or `speed-cli.exe` on Windows).

## Usage

### Traditional TCP/UDP Testing (iperf3-style)

#### Start a Traditional Server

```bash
# Start TCP/UDP server on default port (5201)
speed-cli server

# Start server on specific port and interface
speed-cli server -p 8080 -b 192.168.1.100
```

#### Run Traditional Client Tests

```bash
# Basic TCP test for 10 seconds (default)
speed-cli client -s <server-ip>

# TCP test for specific duration
speed-cli client -s 192.168.1.100 -t 30

# UDP test with target bandwidth
speed-cli client -s 192.168.1.100 -u -b 100

# Export results to file
speed-cli client -s 192.168.1.100 -e results.json
```

### HTTP Speed Tests (Ookla/Cloudflare-style)

#### Start HTTP Test Server

```bash
# Start HTTP server on default port (8080)
speed-cli http-server

# Start server on specific port with custom settings
speed-cli http-server -p 9090 -b 0.0.0.0 --max-upload-mb 500
```

#### Run HTTP Speed Tests

```bash
# Comprehensive HTTP test (includes download, upload, latency)
speed-cli http --url http://localhost:8080

# Download test only with HTTP/2
speed-cli http --url http://localhost:8080 --type download --version http2

# Upload test with multiple parallel connections
speed-cli http --url http://localhost:8080 --type upload --parallel 8

# Bidirectional test with adaptive sizing
speed-cli http --url http://localhost:8080 --type bidirectional --adaptive

# Latency-only test for minimum overhead
speed-cli http --url http://localhost:8080 --type latency-only

# Export HTTP results
speed-cli http --url http://localhost:8080 --export http_results.json
```

### Comprehensive Network Diagnostics

Run the most comprehensive network diagnostic available, essentially a superset of Ookla and Cloudflare tests:

```bash
# Full diagnostic suite (DNS, quality, HTTP performance, topology)
speed-cli diagnostics --url http://localhost:8080

# Quick diagnostics (30 seconds with all tests)
speed-cli diagnostics --url http://localhost:8080 --time 30

# Skip specific test phases
speed-cli diagnostics --url http://localhost:8080 --skip-dns --skip-topology

# Export comprehensive results
speed-cli diagnostics --url http://localhost:8080 --export full_diagnostics.json
```

### Test Against Public Servers

You can test against public HTTP speed test servers:

```bash
# Test against a public server (replace with actual server)
speed-cli http --url https://speed.cloudflare.com

# Comprehensive diagnostics against public server
speed-cli diagnostics --url https://speed.cloudflare.com
```

### Exporting Results

Add a `-e` or `--export` flag to `client` commands to save results in JSON or CSV format:

```bash
# Export to JSON
speed-cli client -s 192.168.1.100 -e results.json

# Export to CSV
speed-cli client -s 192.168.1.100 -e results.csv

# If unknown extension, it assumes JSON
speed-cli client -s 192.168.1.100 -e results.test
```

### Command Reference

```bash
# Show help
speed-cli --help
speed-cli client --help
speed-cli server --help
```

## Comparison with Existing Tools

<!-- TODO: Perhaps remove this -->

| Feature | speed-cli | iperf3 | Ookla Speedtest | Cloudflare Speed Test |
|---------|-----------|--------|-----------------|----------------------|
| TCP Throughput | ✅ | ✅ | ❌ | ❌ |
| UDP Throughput | ✅ | ✅ | ❌ | ❌ |
| HTTP/1.1 Speed Test | ✅ | ❌ | ✅ | ✅ |
| HTTP/2 Speed Test | ✅ | ❌ | ✅ | ✅ |
| Parallel Connections | ✅ | ❌ | ✅ | ✅ |
| DNS Performance | ✅ | ❌ | ❌ | ❌ |
| Jitter Measurement | ✅ | ✅ | ✅ | ✅ |
| Packet Loss Detection | ✅ | ✅ | ✅ | ✅ |
| MTU Discovery | ✅ | ❌ | ❌ | ❌ |
| Network Topology | ✅ | ❌ | ❌ | ❌ |
| Geographic Info | ✅ | ❌ | ✅ | ✅ |
| Bidirectional Testing | ✅ | ❌ | ❌ | ❌ |
| Export Formats | JSON, CSV | ❌ | Limited | Limited |
| Self-Hosted Server | ✅ | ✅ | ❌ | ❌ |
| Cross-Platform | ✅ | ✅ | ✅ | ✅ |

## Example Output

<!-- TODO: Decide whether to remove this -->

<!-- ### HTTP Speed Test Results

```
HTTP Speed Test Results
==================================================
Test Type: Comprehensive
HTTP Version: Auto
Server: http://localhost:8080
Parallel Connections: 4
Test Duration: 30.00s
Download Speed: 1.23 Gbps
Data Downloaded: 4.61 GB
Upload Speed: 987.45 Mbps
Data Uploaded: 3.69 GB
Average Latency: 12.34 ms
Jitter: 2.14 ms
DNS Resolution: 8.45 ms
```

### Comprehensive Diagnostics Results

```
════════════════════════════════════════════════════════════════════════════════
COMPREHENSIVE NETWORK DIAGNOSTIC RESULTS
════════════════════════════════════════════════════════════════════════════════

🎯 Overall Score: 87.5/100

🌐 DNS PERFORMANCE
   Resolution Time: 12.34ms
   IPv4 Support: ✓
   IPv6 Support: ✓
   DNS Server 8.8.8.8: 15.23ms ✓
   DNS Server 1.1.1.1: 11.45ms ✓
   DNS Server 208.67.222.222: 18.67ms ✓

📡 CONNECTION QUALITY
   Jitter: 3.21ms
   Packet Loss: 0.00%
   Stability: Excellent
   Optimal MTU: 1500 bytes

🚀 HTTP PERFORMANCE
   Download Speed: 1.23 Gbps
   Upload Speed: 987.45 Mbps
   HTTP Latency: 12.34ms
   Parallel Connections: 4

🗺️  NETWORK TOPOLOGY
   Hop Count: 8
   Route Stability: Stable

💡 RECOMMENDATIONS
   1. Your network performance looks excellent!
``` -->

## Technical Details

### Supported Protocols

- **TCP**: Stream-based throughput testing
- **UDP**: Packet-based testing with configurable bandwidth
- **HTTP/1.1**: Standard HTTP speed testing
- **HTTP/2**: Modern HTTP with multiplexing support
- **DNS**: Resolution performance testing

### Quality Metrics Calculated

- **Bandwidth**: Measured in Mbps/Gbps
- **Latency**: Round-trip time in milliseconds
- **Jitter**: Standard deviation of latency measurements
- **Packet Loss**: Percentage of lost packets
- **Connection Stability**: Rated from Poor to Excellent
- **DNS Performance**: Resolution time and server comparison

### HTTP Test Endpoints

When running `http-server`, the following endpoints are available:

- `GET /download?size=<bytes>` - Download test data
- `POST /upload` - Upload test endpoint
- `GET /latency` - Minimal latency test
- `GET /health` - Server health check
- `GET /info` - Server information

## Advanced Usage

### Custom Test Scenarios

#### High-bandwidth Testing

```bash
# Test with large parallel connections for high-speed links
speed-cli http --url http://server:8080 --parallel 16 --time 60
```

#### Low-latency Optimization

```bash
# Focus on latency testing
speed-cli http --url http://server:8080 --test-type latency
```

#### Continuous Monitoring

```bash
# Run tests every 5 minutes and log results
while true; do
  speed-cli diagnostics --url http://server:8080 --export "results_$(date +%Y%m%d_%H%M%S).json"
  sleep 300
done
```

### Configuration Files

Create a config file for repeated testing:

```json
{
  "server_url": "http://your-server:8080",
  "test_duration": 30,
  "parallel_connections": 8,
  "export_file": "daily_test.json"
}
```

## Future Improvements

*There are several features/improvements that are planned.*

- [ ] Support multiple incoming client connections (for server)
- [ ] Docker/Kubernetes support (for server)
- [ ] QUIC support (HTTP/3)
- [ ] Rich HTML report generation
- [ ] Incorporate more detailed DNS analysis

## Contributing

Contributions are welcome! This tool aims to be the most comprehensive network testing suite available. Areas for improvement:

- Improve internal overhead of HTTP tests (e.g. to test against 25+ Gbps links)
- Additional protocol support (QUIC, HTTP/3, SFTP, SMB)
- More advanced topology analysis
- Real-time monitoring capabilities
- Web interface for results visualization
- Integration with network monitoring systems

## License

This project is licensed under [AGPL-3.0 License](LICENSE)
