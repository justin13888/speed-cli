# STP (Simple Transport Protocol) Implementation

This document describes the UDP-based transport protocol implementation with BBR congestion control for measuring maximum available bandwidth.

## Overview

- **STP Protocol**: A QUIC-inspired packet-based protocol with reliable acknowledgments
- **BBR Congestion Control**: Modern congestion control algorithm for bandwidth estimation
- **Loss Detection**: Reordering-tolerant loss detection with fast retransmission
- **Precise Pacing**: Rate-based packet transmission to avoid bursts

## Protocol Design

### Packet Header Structure (32 bytes)

Every STP packet includes a fixed 32-byte header:

| Field Name             | Size    | Description                                |
| ---------------------- | ------- | ------------------------------------------ |
| **Packet Number**      | 8 bytes | Monotonically increasing 64-bit identifier |
| **Timestamp**          | 8 bytes | Microsecond timestamp for RTT calculation  |
| **Latest ACK**         | 8 bytes | Highest packet number received from peer   |
| **ACK Timestamp Echo** | 8 bytes | Echo of acknowledged packet's timestamp    |

### Connection Establishment

Simple 2-way handshake:

1. Client sends first data packet (packet number 1)
2. Server responds with ACK, connection established

### Core Protocol Features

#### Data Transmission

- **Congestion-controlled sending**: BBR algorithm controls transmission rate
- **Precise pacing**: Packets are spaced evenly over time based on BBR rate
- **Acknowledgment tracking**: Every data packet is immediately acknowledged

#### Loss Detection and Recovery

- **Reordering threshold**: Packet declared lost after 3 higher packets are ACKed
- **Timeout-based detection**: Fallback timeout for loss detection
- **New packet numbers**: Retransmissions use new packet numbers (QUIC-style)

#### RTT Measurement

- **Timestamp echo**: RTT calculated from original timestamp in ACK
- **Continuous sampling**: Every ACK provides an RTT sample for BBR

## Implementation Architecture

### Core Modules

1. **Protocol Layer** (`protocol.rs`)
   - `StpHeader` and `StpPacket` structures
   - Packet encoding/decoding
   - Connection state management
   - Loss recovery logic

2. **Congestion Control** (`congestion.rs`)
   - `CongestionControl` trait for pluggable algorithms
   - Complete BBR implementation with all phases:
     - Startup: Exponential bandwidth probing
     - Drain: Queue draining after startup
     - ProbeBW: Cyclic bandwidth probing
     - ProbeRTT: Periodic RTT measurement

3. **Pacing** (`pacing.rs`)
   - Rate-based packet transmission
   - Prevents burst sending
   - Integrates with BBR rate estimates

4. **Client** (`client.rs`)
   - `StpClient` implementation
   - Handles data transmission and ACK processing
   - Statistics collection and reporting

5. **Server** (`server.rs`)
   - `StpServer` implementation
   - Immediate ACK responses
   - Per-client session tracking

### BBR Congestion Control Details

The BBR implementation includes:

- **Bandwidth Estimation**: Sliding window maximum of delivery rate samples
- **RTT Tracking**: Minimum RTT over configurable time window
- **State Machine**: Four distinct operating phases
- **Gain Cycling**: Periodic probing for additional bandwidth
- **Pacing Rate**: Direct rate control instead of window-based

#### BBR Parameters

- Startup gain: 2.77 (high gain for fast startup)
- ProbeBW gains: [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0] (cyclic probing)
- Loss threshold: 3 packets (reordering tolerance)
- ProbeRTT duration: 200ms

## Future Enhancements

<!-- TODO: Work through these vv -->

Possible improvements:

1. **Pacing accuracy**: Higher resolution timing for better pacing
2. **BBR v2/v3**: Upgrade to newer BBR variants
3. **ECN support**: Explicit Congestion Notification for better feedback
4. **Connection multiplexing**: Multiple streams over single connection
5. **Adaptive packet sizing**: Dynamic payload size optimization
