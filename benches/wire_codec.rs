//! Micro-benchmarks for the UDP blaster wire codec — the per-packet hot path
//! that runs once for every datagram at line rate.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use speed_cli::performance::udp::protocol::BlasterPacket;

fn wire_codec(c: &mut Criterion) {
    let payload = vec![0xAB_u8; 1200];
    let data = BlasterPacket::Data {
        seq: 42,
        send_ts_us: 1_000_000,
    };

    c.bench_function("blaster_data_encode", |b| {
        b.iter(|| black_box(data.encode_to_vec(Some(black_box(&payload)))));
    });

    let encoded = data.encode_to_vec(Some(&payload));
    c.bench_function("blaster_data_decode", |b| {
        b.iter(|| black_box(BlasterPacket::decode(black_box(&encoded))));
    });
}

criterion_group!(benches, wire_codec);
criterion_main!(benches);
