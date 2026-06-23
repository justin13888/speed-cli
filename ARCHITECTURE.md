# Architecture

`speed-cli` is a single Rust crate built as both a library (`src/lib.rs`, used by
integration tests and embedders) and a thin binary (`src/main.rs`, which just
parses the CLI and dispatches into the library).

## Control plane vs data plane

The tool separates a small **control plane** from the per-protocol **data plane**:

- **Control plane** (`src/control/`): the server binds *every* enabled test
  listener up front — each on its own OS-assigned ephemeral port — then publishes
  a JSON **manifest** over a single HTTP/1.1 control endpoint (default `:9000`).
  The client (or suite) handshakes against that one port, discovers the real
  per-protocol ports, and verifies wire-protocol compatibility before any test
  runs. This is why a user only ever picks the control port.
- **Data plane** (`src/performance/`): one module per protocol (`tcp/`, `udp/`,
  `quic/`, `http/`), each with a client and server half, plus `suite.rs` which
  drives every advertised protocol in sequence for an apples-to-apples report.

See [docs/PROTOCOL.md](docs/PROTOCOL.md) for the wire formats.

## Measurement engine

`src/performance/engine/` is the shared measurement core that every protocol
client builds on:

- **`sampler.rs`** — warmup-aware timing primitives (`offset_us`,
  `measurement_duration_us`).
- **`collector.rs`** — a generic `StatsCollector<T>` that drains samples from an
  unbounded channel into a buffer on one task while a second task refreshes a
  live progress bar. Uses `parking_lot::Mutex`, so a panicked producer cannot
  poison the buffer.
- **`progress.rs`** — progress-bar construction, globally gated (hidden under
  `--quiet` / non-TTY).

Each test produces a stream of **samples** (`report::Sample`): a half-open
interval `[t_start_us, t_start_us + duration_us)` on the test's monotonic time
axis, a byte count, an outcome (success/failure), and a `is_warmup` flag.
Samples taken during the warmup window are excluded from the reported numbers so
results reflect steady state. Aggregation (throughput percentiles, latency
percentiles, jitter) happens at report time.

Latency probes are recorded the same way — one `LatencyMeasurement {
t_start_us, rtt_us }` per probe — so a latency result is a time-line, not just a
summary. On top of that series the report derives tail percentiles, adaptive
**spike detection**, and a time-vs-latency chart (a terminal sparkline plus a
self-contained SVG in the HTML report). The UDP `latency-under-load` test type
(`--type latency-load`) drives this end to end: it captures an idle baseline,
then probes latency at ~200 Hz while saturating the link, and reports the
idle-vs-loaded "bufferbloat" inflation — stored in the report's
`latency_under_load` field (added in schema version 6) and the WiFi / AP
latency-spike signal.

## Reports

`src/report/` defines the result types and serialization:

- `TestReport` carries the environment snapshot, the two peers' identities, the
  test configuration, and the per-stream samples. It is versioned by
  `REPORT_SCHEMA_VERSION`; imports reject any other version (no backward-compat
  shims — CBOR is binary and compact).
- `src/utils/{export,import}.rs` handle CBOR (the canonical, re-importable
  format) and `src/renderer/` renders a self-contained HTML report. There is no
  JSON report format by design.

## Output discipline

The report (and "exported to X" confirmations) go to **stdout**; every log line,
progress bar, and diagnostic goes to **stderr** via `tracing`
(`src/utils/logging.rs`). So `speed-cli client ... > out.cbor` captures only the
report. Verbosity (`-v/-vv/-q`, `RUST_LOG`) and color (`--color`, `NO_COLOR`) are
global flags.

## Tooling

Tasks and tooling are managed by [mise](https://mise.jdx.dev); git hooks by
[hk](https://hk.jdx.dev). CI (`.github/workflows/`) runs the static gate
(`fmt`, `clippy -D warnings`, `typos`, `cargo-machete`) plus the test suite on
Linux/macOS/Windows, and release-plz automates versioned GitHub Releases. See
[CONTRIBUTING.md](CONTRIBUTING.md).
