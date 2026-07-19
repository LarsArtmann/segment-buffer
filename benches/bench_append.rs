//! Benchmark: append throughput at varying batch sizes.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

fn item(n: u64) -> Item {
    Item {
        id: n,
        payload: format!("payload-{n}"),
    }
}

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("append");
    for batch_size in [1usize, 100, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(batch_size as u64));
        group.bench_function(format!("batch_{batch_size}"), |b| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    let buf = SegmentBuffer::<Item>::open(
                        tmp.path(),
                        SegmentConfig {
                            max_batch_events: 100_000, // don't auto-flush mid-bench
                            flush_interval_secs: 3600,
                            max_size_bytes: u64::MAX,
                            compression_level: 3,
                            cipher: None,
                        },
                    )
                    .unwrap();
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    for i in 0..batch_size as u64 {
                        buf.append(item(i)).unwrap();
                    }
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
