//! Benchmark: read_from throughput with varying limits.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

fn populate(buf: &SegmentBuffer<Item>, n: u64) {
    for i in 0..n {
        buf.append(Item {
            id: i,
            payload: format!("payload-{i}"),
        })
        .unwrap();
    }
    buf.flush().unwrap();
}

fn bench_read_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_from");
    for limit in [100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(limit as u64));
        group.bench_function(format!("limit_{limit}"), |b| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    let buf = SegmentBuffer::<Item>::open(
                        tmp.path(),
                        SegmentConfig {
                            max_batch_events: 100_000,
                            flush_interval_secs: 3600,
                            max_size_bytes: u64::MAX,
                            compression_level: 3,
                            cipher: None,
                        },
                    )
                    .unwrap();
                    populate(&buf, 10_000);
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    let events = buf.read_from(0, limit).unwrap();
                    black_box(events.len());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_read_from);
criterion_main!(benches);
