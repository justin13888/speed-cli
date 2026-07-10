# Roadmap

Planned work and — just as importantly — decisions that were made
deliberately and should not be re-litigated by accident. Code comments
point here so the rationale has a stable home.

## Planned

- [ ] OCI container images using all popular base images (necessary for
      representative performance testing)
- [ ] Kubernetes support (for server)
- [ ] gRPC support?
- [ ] Support for more niche protocols (e.g. SFTP, SMB)
- [ ] Remote SSH server spin-up (remote host downloads the binary, or
      receives it from the client, and runs a server as specified by
      the client)
- [ ] Mobile app support (iOS/Android)
- [ ] Firm up IPv6 support (which has different NAT characteristics)
- [ ] QUIC-based congestion-controlled UDP mode (a paced UDP test that
      reacts to loss, unlike the fixed-rate blaster)

## Deliberately deferred

Each entry records *why* it is not done and *when to revisit*, so the
next reader doesn't have to re-derive the trade-off.

### io_uring / monoio (or other io_uring runtimes)

**Why not now:** Linux-only, so it fragments the I/O stack across
platforms; the io_uring ecosystem crates require `unsafe` at the
boundary, conflicting with this crate's `unsafe_code = "forbid"`
posture; and tokio's epoll stack is not the bottleneck at the link
rates the tool currently targets — profiling (docs/PROFILING.md) shows
the hot paths are payload handling and the network itself well past
10 Gbps loopback.

**Revisit when:** targeting sustained 25–40 Gbps single-host rates or
very high request rates, where per-syscall overhead dominates. Pairs
with kTLS below — the gains multiply when the NIC offloads TLS too.
Effort is likely better spent on the client side first (servers can
scale horizontally).

### kTLS

**Why not now:** rustls kTLS offload is immature, and without io_uring
the syscall savings are partial.

**Revisit when:** rustls/kTLS integration stabilizes; evaluate together
with io_uring.

### musl static release targets

**Why not now:** HTTP/3 (reqwest's unstable feature) + aws-lc-rs are
fragile to build on musl / cross targets (see the matrix comment in
`.github/workflows/release-binaries.yml`).

**Revisit when:** those build issues are confirmed fixed. When musl
lands, the `mimalloc` default feature becomes load-bearing rather than
nice-to-have: musl's default malloc is slow under multithreaded
allocation, and the allocator is the classic musl performance trap.
Porting the HTTP/3 client off reqwest (tracked separately) would also
unblock this.

### TCP BBR

**Why not:** excluded by design, not deferred. TCP congestion control
is an OS/sysctl concern (`net.ipv4.tcp_congestion_control`) — there is
no portable per-socket knob, and silently depending on host sysctls
would make results non-comparable across machines. The environment
snapshot records the host's TCP CC in every report, and the QUIC-based
tests expose `--congestion bbr` (per-listener, symmetric both
directions) for measuring BBR behavior portably.

### SO_REUSEPORT

**Why not:** excluded by design. Its purpose is multi-accept-loop
scaling, which this tool does not need; and it would allow a second
server instance to silently bind the same port and steal a fraction of
connections — a correctness hazard for a measurement tool. Fast restart
is covered by SO_REUSEADDR (set on Unix TCP listeners; see
`src/utils/net.rs`).

## Historical context

### Retired: STP / BBR UDP implementation

The original UDP test implemented a custom congestion-controlled
protocol ("STP", with a BBR-style controller). It was retired in favor
of the current iperf3-u-style fixed-rate blaster: a measurement tool
wants a *predictable* offered load so loss/jitter numbers are
attributable to the network, not to the sender's controller. The
"congestion-controlled UDP" itch is better scratched by the raw-QUIC
test (a real, interoperable congestion-controlled transport) and the
planned QUIC-based UDP mode above.

### UDP pacing granularity

The blaster paces with `tokio::time::sleep` (~1 ms resolution), so
pacing above ~100 Mbps per stream is approximate; parallel streams
raise the aggregate ceiling, and `--target-rate-mbps 0` (saturate)
sidesteps pacing entirely. A timer-wheel or busy-wait pacer is possible
but has not been worth the CPU cost so far.
