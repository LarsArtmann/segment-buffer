//! Benchmark: delete_acked throughput after populating segments.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

fn bench_delete_acked(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_acked");
    group.throughput(criterion::Throughput::Elements(100));
    group.bench_function("100_segments", |b| {
        b.iter_with_setup(
            || {
                let tmp = tempfile::tempdir().unwrap();
                let buf = SegmentBuffer::<Item>::open(
                    tmp.path(),
                    SegmentConfig {
                        max_batch_events: 4, // small to create many segment files
                        flush_interval_secs: 3600,
                        max_size_bytes: u64::MAX,
                        compression_level: 3,
                        cipher: None,
                    },
                )
                .unwrap();
                for i in 0..400 {
                    buf.append(Item {
                        id: i,
                        payload: format!("payload-{i}"),
                    })
                    .unwrap();
                }
                (buf, tmp)
            },
            |(buf, _tmp)| {
                // Ack the first 50 segments (200 events of 400)
                let deleted = buf.delete_acked(199).unwrap();
                black_box(deleted);
            },
        )
    });
    group.finish();
}

criterion_group!(benches, bench_delete_acked);
criterion_main!(benches);
