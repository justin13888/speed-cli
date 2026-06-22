# speed-cli

> Disclaimer: Tool is under active development. Some features are to be improved in correctness/performance and documentation. Open to contributions!

This tool provides **comprehensive network performance measurements** across **TCP, UDP, and HTTP** protocols. Built with Rust, the test suite optimizes for **maximum throughput** to measure the network's available bandwidth.

## Why Another Network Testing Tool?

It's difficult to have one tool that tests your network conditions between two devices in a way that is representative of real-world traffic. Most tools focus and excel at specific aspects (e.g. iperf3 with TCP/UDP, speed test cools for HTTP throughput/latency). However, applications use a mixture of protocols with various characteristics. This tool is a framework to create synthetic but realistic network loads between any two devices, on any platform using [Rust](https://www.rust-lang.org/).

## Features

- **Multi-protocol support**: TCP, UDP, raw QUIC, HTTP/1.1, HTTP/2, h2c, HTTP/3
- **High-performance**: Built with Rust, optimized for high throughput and efficient resource usage
- **Comprehensive metrics**: throughput (goodput/wire), latency percentiles, jitter, packet loss
- **Exporting**: CBOR for re-importable data; HTML for self-contained rendered reports
- **Cross-platform**: Optimized for popular platforms (Linux, macOS, Windows) and architectures (x86_64, ARM)

## Installation

Build from source:

```bash
# Prerequisite: Rust installed via [rustup](https://rustup.rs/)
git clone https://github.com/justin13888/speed-cli
cd speed-cli
cargo install --path .
```

The binary name is `speed-cli`. Note, for the HTTPS server, you may provide your own TLS certificate and key files via `--cert` and `--key`, or else a dummy cert will be used. A convenience script `./gen-cert.sh` is provided to generate self-signed certificates for testing purposes. This is not suitable for production use.

> Note on HTTPS measurements: the client always skips TLS certificate validation (`danger_accept_invalid_certs`) so that self-signed test servers work out of the box. Reported HTTPS throughput and latency therefore exclude the cost of certificate-chain verification, OCSP, and revocation checks. If you need numbers that reflect production TLS overhead, validate against a real cert with another tool.

## Quick Start

Note: If you're using HTTPS server, ensure you have `cert.pem` and `key.pem` files in the current directory or specify them with `--cert` and `--key` flags.

The server publishes a single JSON **control endpoint** (default port
`9000`). Every enabled protocol binds its own OS-assigned ephemeral
port; clients discover those ports — and verify wire-protocol
compatibility — by handshaking against the control port. The control
port is the only port you normally choose.

```sh
# Start server (control endpoint on 9000; every test listener ephemeral)
speed-cli server --all                                 # all protocols
speed-cli server --protocol tcp --protocol udp         # selected protocols only
speed-cli server --all --control-port 9100             # pick a different control port
speed-cli server --all -b 192.168.1.100                # bind a specific interface

# Run a single-protocol client test (only --server + --control-port needed)
speed-cli client --protocol tcp   -s <server-ip>       # raw TCP
speed-cli client --protocol udp   -s <server-ip>       # UDP blaster
speed-cli client --protocol quic  -s <server-ip>       # raw QUIC streams
speed-cli client --protocol http1 -s <server-ip>       # HTTP/1.1
speed-cli client --protocol http2 -s <server-ip>       # HTTP/2 (TLS)
speed-cli client --protocol h2c   -s <server-ip>       # HTTP/2 cleartext
speed-cli client --protocol http3 -s <server-ip>       # HTTP/3 (over QUIC)

# Longer test, more connections, export to CBOR
speed-cli client --protocol http1 -s 192.168.1.100 -d 60 -c 4 -e results.cbor

# Run the full multi-protocol suite (drives every advertised protocol)
speed-cli suite -s <server-ip>
speed-cli suite -s <server-ip> --control-port 9100 -e suite.cbor

# Print a previously saved result
speed-cli report -f results.cbor
```

For more advanced usage, refer to help:

```sh
speed-cli -h
speed-cli client -h
speed-cli server -h

# Verbosity / color are global: -v (debug), -vv (trace), -q (quiet), --color never
# Shell completions and man pages:
speed-cli completions zsh > _speed-cli   # bash | zsh | fish | powershell | elvish
speed-cli man --out-dir ./man
```

### Exporting Results

Add a `-e` or `--export` flag to `client` commands to save results.
The data format is CBOR — there is no JSON export. HTML is available
as a rendered single-file report for visual inspection.

```bash
# Export raw data (re-importable)
speed-cli client --protocol <p> -s <server-ip> -e results.cbor

# Export rendered HTML report
speed-cli client --protocol <p> -s <server-ip> -e results.html

# No extension implies CBOR
speed-cli client --protocol <p> -s <server-ip> -e results
```

Other extensions (`.json`, `.txt`, …) are rejected with a clear error.

## Developer Notes

### HTTP Test Endpoints

When running server with HTTP, the following endpoints are available:

- `GET /download?size=<total_size>&chunk_size=<chunk_size>` - Download test data
- `POST /upload` - Upload test endpoint
- `GET /latency` - Minimal latency test
- `GET /info` - Server information
- `GET /health` - Server health check

### UDP Test Implementation

The UDP test uses a small, iperf3-u-style "blaster" protocol: a fixed-rate sender, no retransmissions, server-side counting of received / lost / out-of-order packets and RFC 3550 interarrival jitter. Use `--target-rate-mbps <N>` to pace at a specific rate, or leave it at the default `0` to saturate. Pacing uses `tokio::time::sleep` and is therefore approximate above ~100 Mbps; for higher rates either accept the bursting or shape externally with `tc fq`. A QUIC-based congestion-controlled UDP mode is on the roadmap.

## Future Improvements

*There are several features/improvements that are planned.*

- [ ] OCI Container images using all popular base images (necessary for representative performance testing)
- [ ] Kubernetes support (for server)
- [x] QUIC support (HTTP/3 and raw QUIC streams)
- [ ] gRPC support?
- [ ] Rich HTML report generation
- [ ] Support for more niche protocols (e.g. SFTP, SMB)
- [ ] Remove SSH server spin-up (remote SSH server downloads binary, or through client, and runs server based on what's specified by client)
- [ ] Mobile app support (iOS/Android)
- [ ] Firm up IPV6 support (which has different NAT characteristics)

## Development

This repo uses [mise](https://mise.jdx.dev) for tooling/tasks and [hk](https://hk.jdx.dev)
for git hooks. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

```bash
mise install && mise run setup   # install pinned tools + git hooks
mise run check                   # fmt, clippy -D warnings, typos, unused-deps
mise run test                    # nextest + doctests
mise run bench                   # criterion benchmarks
```

## Contributing

Contributions are welcome! This tool aims to be the most comprehensive network testing suite available. Areas for improvement:

- Improve internal overhead of HTTP tests (e.g. to test against 25+ Gbps links)
- Additional protocol support (QUIC, HTTP/3, SFTP, SMB)
- More advanced topology analysis
- Real-time monitoring capabilities
- Web interface for results visualization
- Integration with network monitoring systems

## License

This project is licensed under the [Apache License 2.0](LICENSE).
