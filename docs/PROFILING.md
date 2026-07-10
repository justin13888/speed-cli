# Profiling & benchmarking

How to find out where speed-cli itself spends time — for making sure the
client and server are never the bottleneck of a measurement — and how to
run controlled A/B comparisons of build configurations (allocators,
features, flags).

## Build profiles

| Profile | What it's for |
|---|---|
| `release` | Shipped binaries. Fully optimized (`lto`, `codegen-units = 1`) and **stripped** — useless for profilers. |
| `profiling` | Same codegen as release, but `debug = true` and `strip = false` so perf/dtrace/flamegraph can symbolize stacks. Build with `cargo build --profile profiling`. |

Never profile a `dev` build: unoptimized code has a completely different
hot-path shape (bounds checks, no inlining) and will send you optimizing
the wrong thing. Note the release profile inherits `panic = "abort"`.

## Flamegraphs

Uses [cargo-flamegraph](https://github.com/flamegraph-rs/flamegraph); the
mise tasks below fetch it automatically as a per-task tool (or
`cargo install flamegraph` yourself).

Platform prerequisites:

- **Linux**: `perf` (usually the `perf` / `linux-tools` package). Unprivileged
  profiling needs `kernel.perf_event_paranoid` ≤ 1
  (`sudo sysctl -w kernel.perf_event_paranoid=1`), or run the task with
  `--root`.
- **macOS**: dtrace (ships with the OS); usually needs `sudo`.

Typical session — profile the server while a suite drives it:

```sh
# Terminal 1: server under the profiler (Ctrl-C when done -> flamegraph.svg)
mise run flamegraph-server

# Terminal 2: generate load
cargo run --release -- suite -s 127.0.0.1 -d 30
```

Or profile the client side against an already-running server:

```sh
mise run flamegraph-client -- -d 30        # extra args go to `speed-cli suite`
```

Open the resulting `flamegraph.svg` in a browser. Wide plateaus inside
`speed_cli::performance::*` are candidates; wide plateaus in
`memcpy`/syscalls tell you the tool is I/O-bound (which, for a network
measurement tool, is the goal state).

## Loopback A/B benchmarking

`scripts/loopback-bench.sh` (also `mise run bench-loopback`) builds the
release binary, starts a loopback server, runs the full suite against it,
and saves a labeled CBOR report under `target/`. Loopback removes the
network, so what remains is the software stack — exactly what you want
when comparing build configurations.

Allocator A/B (mimalloc is a default feature; the escape hatch disables it):

```sh
LABEL=mimalloc  DURATION=30 ./scripts/loopback-bench.sh
LABEL=sysmalloc DURATION=30 ./scripts/loopback-bench.sh --no-default-features
```

Methodology, so the numbers mean something:

- Use **30 s+ phases** (`DURATION`); short runs are dominated by warmup and
  scheduler noise.
- Run each variant **at least 3 times, interleaved** (A B A B A B), and
  compare **medians** — thermal drift and background tasks bias
  back-to-back runs.
- Keep the machine otherwise idle; on laptops, pin the power profile.
- Render any report for inspection:
  `target/release/speed-cli report -f target/loopback-<label>-<ts>.cbor --export-html out.html`.

Caveats: loopback throughput is CPU-bound and per-machine; it says nothing
about real network behavior (congestion control never engages
meaningfully, RTT is ~0). Use it for *relative* comparisons of the tool's
own overhead, never as a bandwidth claim.

## Microbenchmarks

`mise run bench` runs the criterion microbenchmarks (`benches/`), currently
covering the UDP blaster wire codec. Prefer a criterion bench when the
question is "how fast is this function", and the loopback harness when the
question is "does this change move end-to-end numbers".
