# Wire protocol

This documents the on-the-wire formats `speed-cli` uses. The control
`PROTOCOL_VERSION` is `1`; a client refuses to test against a server advertising
a different version.

## Control endpoint (HTTP/1.1)

The server exposes one control endpoint on `--control-port` (default `9000`):

- `GET /manifest` (also `GET /`) → the **manifest** as JSON: the protocol
  version, the report schema version, and one entry per enabled listener
  (`{ transport, host, port, congestion }`). The client uses this to discover
  the real (usually ephemeral) per-protocol ports and to verify compatibility.
- `GET /health` → `ok`.

### The `congestion` listener field

`congestion` (`"cubic"` or `"bbr"`) names the congestion controller a listener
runs with. It only matters for the QUIC transports (`quic-raw`, `http3`): a
server binds **one endpoint per algorithm** for each of those and advertises
the same transport twice with different `congestion` values and ports. A client
picks the algorithm by dialing the matching port (and configures its own QUIC
transport to match), so both directions of a test use the same controller
without any per-connection negotiation.

Compatibility rules (why this needs no protocol-version bump):

- A missing `congestion` field means `cubic` — manifests from old servers stay
  valid.
- For a given transport, the `cubic` entry is always listed **before** any
  `bbr` entry. Old clients resolve listeners first-match, so they keep landing
  on cubic. This ordering is a compatibility guarantee, not an implementation
  detail.
- A client that wants `bbr` from a server that does not advertise it gets a
  hard error before any test runs, never a silent cubic test.

TCP-based transports carry `"cubic"` as an inert placeholder; there is no
portable per-socket TCP congestion knob, so no TCP listener variants exist.

## Identity handshake (`'H'`)

On the stream-oriented transports (raw TCP, raw QUIC) the first thing a client
does on a test connection is a transport-agnostic identity exchange so each side
records the other's identity and observed address:

```
client → server:  'H' (1 byte) │ u32-BE len │ CBOR(PeerIdentity)
server → client:  u32-BE len │ CBOR(PeerIdentity) │ u32-BE len │ CBOR(SocketAddr)
```

Blobs are capped (16 KiB) to bound allocation. Over UDP the same information is
carried by the `HELLO` / `HELLO_ACK` blaster packets (below).

## Raw TCP

After the optional `'H'` handshake, the client sends a 1-byte command selecting
the test mode, then streams data:

| Byte | Mode |
|------|------|
| `'D'` | download (server sends) |
| `'U'` | upload (client sends) |
| `'F'` | full-duplex (both directions on one connection) |
| `'P'` | latency ping/pong |

Raw QUIC mirrors this over bidirectional streams (one command per stream),
negotiated with a dedicated ALPN.

## UDP blaster

An iperf3-u-style fixed-rate protocol: no retransmissions; the receiver counts
received / lost / out-of-order / duplicate packets and computes RFC 3550
interarrival jitter. Every packet starts with a 32-bit magic `0x424C5354`
(`"BLST"`) and a 1-byte kind:

| Kind | Name | Payload (after magic+kind) |
|------|------|-----|
| 1 | `START` | mode(u8), pad(2), target_rate_bps(u64), payload_size(u32), duration_ms(u64) |
| 2 | `DATA` | pad(3), seq(u64), send_ts_us(u64), then the payload |
| 3 | `FIN` | — |
| 4 | `REPORT` | pad(3), received(u64), bytes_received(u64), lost(u64), out_of_order(u64), jitter_us(u64), duplicates(u64) |
| 5 / 6 | `PING` / `PONG` | pad(3), send_ts_us(u64) |
| 7 | `HELLO` | pad(3), t_send_us(u64), id_len(u16), CBOR(PeerIdentity) |
| 8 | `HELLO_ACK` | pad(3), server_epoch_us(u64), id_len(u16), CBOR(PeerIdentity), addr_len(u16), CBOR(SocketAddr) |

Timestamps are `SystemTime` microseconds. Jitter only ever uses the *difference*
of consecutive one-way transit samples, so the constant clock offset between the
two hosts cancels (per RFC 3550) and no clock synchronization is required.
Decoding tolerates a missing trailing `duplicates` field so newer servers can
report to older peers.

## HTTP (1.1 / h2c / HTTP/2-TLS / HTTP/3)

The HTTP servers expose:

- `GET /download?size=<bytes>&chunk_size=<bytes>` — streamed synthetic body.
- `POST /upload` — body is drained and measured.
- `GET /latency` — minimal response for RTT probing.
- `GET /info` — server info; the server identity travels in the
  `x-speed-cli-server-id` response header as base64url-encoded CBOR.
- `GET /health` — health check.

`http` on the server enables both HTTP/1.1 and h2c (each on its own port);
`https` is HTTP/2 over TLS; `http3` is HTTP/3 over QUIC. The HTTPS/HTTP/3/raw-QUIC
listeners share one TLS certificate (self-signed by default).
