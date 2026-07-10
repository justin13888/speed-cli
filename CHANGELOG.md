# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).
From v1.0.0 onward, releases are automated by
[release-plz](https://release-plz.dev) from Conventional Commit messages; this
first entry is hand-written because the pre-1.0 history predates that convention.

## [2.0.0](https://github.com/justin13888/speed-cli/compare/v1.0.0...v2.0.0) - 2026-07-10

### Added

- use mimalloc as the global allocator (default feature)
- *(net)* SO_REUSEADDR on TCP listeners, opt-in --socket-buffer, kernel clamp hints
- *(quic)* optional BBR congestion control for QUIC and HTTP/3 tests
- *(report)* render suite reports to HTML and auto-detect report type on import
- *(report)* report requests-per-second for HTTP bandwidth tests
- *(report)* record per-protocol handshake and TTFB timings
- *(udp)* size socket send/receive buffers

### Fixed

- *(perf)* tag throughput warmup at request completion, not start
- detect hostname on macOS via hostname command fallback

### Other

- add ROADMAP.md and PROFILING.md links; fix dangling roadmap references
- add profiling profile, flamegraph tasks, and loopback bench harness
- satisfy rustfmt and clippy on Rust 1.96
- *(perf)* cover the simultaneous HTTP full-duplex path
- remove remaining buffer copies
- bump schema-version tripwire to 9
- *(udp)* batch send via GSO and receive via GRO
- *(udp)* drop per-packet session lock in download sender
- *(udp)* decode packets without copying the datagram
- *(contributing)* document trusted publishing and the Actions PR setting
- *(release-plz)* publish via crates.io trusted publishing, drop publish tokens
- *(release-binaries)* allow invocation via workflow_call with a tag input

## [1.0.0] - 2026-06-25

First stable release.

### Added

- **Multi-protocol testing** — throughput, latency, jitter, and loss across TCP,
  UDP, raw QUIC, HTTP/1.1, HTTP/2 (TLS), h2c (cleartext HTTP/2), and HTTP/3.
- **Control handshake** — a single JSON control endpoint advertises every enabled
  test listener on an OS-assigned ephemeral port; clients discover those ports and
  verify wire-protocol compatibility (`PROTOCOL_VERSION`) before running, aborting
  on a mismatch.
- **Test types** — download, upload, bidirectional, full-duplex, latency, and
  latency-under-load (WiFi / bufferbloat) stress testing.
- **Multi-stream UDP blaster** — iperf3-u-style fixed-rate sender with server-side
  received / lost / out-of-order accounting, RFC 3550 interarrival jitter, and
  `--target-rate-mbps` pacing.
- **Throughput metrics** — goodput and on-the-wire throughput, with windowed
  percentiles.
- **Latency analysis** — tail percentiles (p95 / p99 / p99.9), adaptive spike
  detection, and a time-vs-latency chart (terminal sparkline + SVG in HTML reports).
- **Suite mode** — drives every advertised protocol with unified phase parameters
  for apples-to-apples comparison.
- **Reporting** — re-importable CBOR and self-contained HTML reports (report
  schema v8).
- **Build provenance** — version, git commit, rustc, and OS / arch reported across
  the CLI and in peer identity.
- **Distribution** — cross-platform release binaries (Linux, macOS, Windows;
  x86_64 and ARM), shell completions, and man-page generation.

[1.0.0]: https://github.com/justin13888/speed-cli/releases/tag/v1.0.0
