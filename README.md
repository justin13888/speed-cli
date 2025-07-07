# speed-cli

## Why another network testing tool?

It's difficult to have one tool that tests your network conditions between two devices in a way that is representative of real-world traffic. Most tools focus and excel at specific aspects (e.g. iperf3 with TCP/UDP, speed test cools for HTTP throughput/latency). However, applications use a mixture of protocols with various characteristics. This tool is a framework to create synthetic but realistic network loads between any two devices, on any platform using [Rust](https://www.rust-lang.org/).

## Features

- Bandwidth
- Jitter
- Download
- Upload
- MSS/MTU Size
- UDP
  - Bandwidth
  - Packet loss
  - [Delay jitter](https://en.wikipedia.org/wiki/Packet_delay_variation)

- Export data (CSV, JSON)
- Render report (HTML)

## Installation

Build from source:
```bash
git clone https://github.com/justin13888/speed-cli
cd speed-cli
cargo build --release
```

The binary will be available at `target/release/speed-cli` (or `speed-cli.exe` on Windows).

## Usage

### Basic Usage

Start a server:
```bash
speed-cli server
```

Run a client test (TCP by default):
```bash
speed-cli client -s <server-ip>
```

### TCP Bandwidth Test

```bash
# Test against a server for 10 seconds (default)
speed-cli client -s 192.168.1.100

# Test for a specific duration
speed-cli client -s 192.168.1.100 -t 30

# Test on a different port
speed-cli client -s 192.168.1.100 -p 8080
```

### UDP Bandwidth Test

```bash
# UDP test with target bandwidth of 1 Mbps (default)
speed-cli client -s 192.168.1.100 -u

# UDP test with target bandwidth of 100 Mbps
speed-cli client -s 192.168.1.100 -u -b 100

# UDP test with export to JSON
speed-cli client -s 192.168.1.100 -u -b 50 -e results.json
```

### Server Mode

```bash
# Start server on default port (5201)
speed-cli server

# Start server on specific port and interface
speed-cli server -p 8080 -b 192.168.1.100
```

### Export Results

Export test results to JSON or CSV:

```bash
# Export to JSON
speed-cli client -s 192.168.1.100 -e results.json

# Export to CSV
speed-cli client -s 192.168.1.100 -e results.csv

# Auto-detect format based on extension
speed-cli client -s 192.168.1.100 -e mytest.json
```

### Command Reference

```bash
# Show help
speed-cli --help
speed-cli client --help
speed-cli server --help
```

## Future Improvements

*There are several features/improvements that are planned.*

- [ ] Docker/Kubernetes support (for server)

## License

This project is licensed under [AGPL-3.0 License](LICENSE)
