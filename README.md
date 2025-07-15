# speed-cli

> Disclaimer: Tool is under active development. Some features are to be improved in correctness/performance and documentation. Open to contributions!

This tool provides **comprehensive network performance measurements** across **TCP, UDP, and HTTP** protocols. Built with Rust, it prioritizes **maximum performance** to help you isolate and identify network or infrastructure-related bottlenecks.

## Why Another Network Testing Tool?

It's difficult to have one tool that tests your network conditions between two devices in a way that is representative of real-world traffic. Most tools focus and excel at specific aspects (e.g. iperf3 with TCP/UDP, speed test cools for HTTP throughput/latency). However, applications use a mixture of protocols with various characteristics. This tool is a framework to create synthetic but realistic network loads between any two devices, on any platform using [Rust](https://www.rust-lang.org/).

## Features

- **Multi-protocol support**: TCP, UDP, HTTP/1.1, HTTP/2, HTTP/3
- **High-performance**: Built with Rust, optimized for high throughput and efficient resource usage
- **Comprehensive metrics**: Throughput, latency, jitter, packet loss, DNS performance
- **Exporting**: Results in JSON and HTML formats
- **Cross-platform**: Optimized for popular platforms (Linux, macOS, Windows) and architectures (x86_64, ARM)

## Installation

Build from source:

```bash
# Prerequisite: Rust installed via [rustup](https://rustup.rs/)
git clone https://github.com/justin13888/speed-cli
cd speed-cli
cargo install --path .
```

The binary name is `speed-cli`. Note, for the HTTPS server, you may need to provide your own TLS certificate and key files, or they are assumed to be `cert.pem` and `key.pem` in the current directory. A convenience script `./gen-cert.sh` is provided to generate self-signed certificates for testing purposes. This is not suitable for production use.

## Quick Start

Note: If you're using HTTPS server, ensure you have `cert.pem` and `key.pem` files in the current directory or specify them with `--cert` and `--key` flags.

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

## Developer Notes

### HTTP Test Endpoints

When running server with HTTP, the following endpoints are available:

- `GET /download?size=<total_size>?chunk=<chunk_size>` - Download test data
- `POST /upload` - Upload test endpoint
- `GET /latency` - Minimal latency test
- `GET /info` - Server information
- `GET /health` - Server health check

## Future Improvements

*There are several features/improvements that are planned.*

- [ ] Updated `--json` output format
- [ ] Support multiple incoming client connections (for server)
- [ ] OCI Container images using all popular base images (necessary for representative performance testing)
- [ ] Kubernetes support (for server)
- [ ] HTTPS support for HTTP tests
- [ ] QUIC support (HTTP/3)
- [ ] gRPC support?
- [ ] Rich HTML report generation
- [ ] Incorporate more detailed DNS analysis
- [ ] Support for more niche protocols (e.g. SFTP, SMB)
- [ ] Remove SSH server spin-up (remote SSH server downloads binary, or through client, and runs server based on what's specified by client)
- [ ] Mobile app support (iOS/Android)
- [ ] Firm up IPV6 support (which has different NAT characteristics)

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
