# speed-cli

> Disclaimer: Tool is under active development. Some features are to be improved in correctness/performance and documentation. Open to contributions!

This tool provides **comprehensive network performance measurements** across **TCP, UDP, and HTTP** protocols. Built with Rust, it prioritizes **maximum performance** to help you isolate and identify network or infrastructure-related bottlenecks.

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

- **Throughput** (upload/download)
- **Latency** (round-trip time)
- **Jitter** (latency variation)
- **Packet Loss**
- **Connection Stability**
- **DNS Resolution Performance**

### Export and Reporting

- **Multiple formats:** JSON, HTML
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

## Quick Start

```sh
# Start server on default port
speed-cli server --all # All protocols (TCP, UDP, HTTP, HTTPS)
speed-cli server --tcp # TCP on default port 5201
speed-cli server --udp # UDP on default port 5201
speed-cli server --http # HTTP on default port 8080
speed-cli server --https # HTTPS on default port 8443

# Run server with specific port and interface
speed-cli server --http -p 8080 -b 192.168.1.100

# Run client test (with defaults)
speed-cli client --tcp -s <server-ip> # TCP test
speed-cli client --udp -s <server-ip> # UDP test
speed-cli client --http1 -s <server-ip> # HTTP/1.1 test
speed-cli client --http2 -s <server-ip> # HTTP/2 test
speed-cli client --h2c -s <server-ip> # HTTP/2 cleartext test
speed-cli client --http3 -s <server-ip> # HTTP/3 test

# Run HTTP client test against specific server for 60 seconds
speed-cli client --http -p 8080 -h 192.168.1.100 -d 60

# Run HTTP client test with 8 parallel connections, and export results to JSON
speed-cli client --http -p 8080 -h 192.168.1.100 --parallel 4 -e results.json

# Run TCP client test against specific server
speed-cli client --tcp -p 5201 -h 192.168.1.100

# # Run full network diagnostics (server should be running with all protocols enabled `-a`)
# speed-cli diagnostics -h 192.168.1.100 --http-port 8080 --tcp-port 5201 --udp-port 5201

# Print previously saved result
speed-cli report -f results.json
```

For more advanced usage, refer to help:

```sh
speed-cli -h
speed-cli client -h
speed-cli server -h
```

### Exporting Results

Add a `-e` or `--export` flag to `client` commands to save results in JSON or HTML format:

```bash
# Export to JSON
speed-cli client --<mode> -s <server-ip> -e results.json

# Export to HTML
speed-cli client --<mode> -s <server-ip> -e results.html

# If unknown extension, it assumes JSON
speed-cli client --<mode> -s <server-ip> -e results.test
```

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

## Technical Details

<!-- TODO: Update this whole section vv -->

### Supported Protocols

<!-- TODO: Need to clarify these descriptions -->

- **TCP**: Stream-based throughput testing
- **UDP**: Packet-based testing
- **HTTP/1.1**: Standard HTTP speed testing
- **HTTP/2**: Modern HTTP with multiplexing support

### Quality Metrics Calculated

- **Throughput**: Measured in Mbps/Gbps
- **Latency**: Round-trip time in milliseconds
- **Jitter**: Standard deviation of latency measurements
- **Packet Loss**: Percentage of lost packets

### HTTP Test Endpoints

<!-- TODO: Update this whole section vv -->

When running `http-server`, the following endpoints are available:

- `GET /download?size=<bytes>` - Download test data
- `POST /upload` - Upload test endpoint
- `GET /latency` - Minimal latency test
- `GET /health` - Server health check
- `GET /info` - Server information

## Future Improvements

*There are several features/improvements that are planned.*

- [ ] Support multiple incoming client connections (for server)
- [ ] OCI Container images using all popular base images (necessary for representative performance testing)
- [ ] Kubernetes support (for server)
- [ ] HTTPS support for HTTP tests
- [ ] QUIC support (HTTP/3)
- [ ] gRPC support?
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
