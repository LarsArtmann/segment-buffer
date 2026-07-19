//! Benchmark: crash recovery throughput — SegmentBuffer::open() on a directory
//! with pre-existing segment files.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

fn bench_recover(c: &mut Criterion) {
    let mut group = c.benchmark_group("recover");
    for n_segments in [10usize, 100, 1_000] {
        group.throughput(criterion::Throughput::Elements(n_segments as u64));
        group.bench_function(format!("{n_segments}_segments"), |b| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    {
                        let buf = SegmentBuffer::<Item>::open(
                            tmp.path(),
                            SegmentConfig {
                                max_batch_events: 4,
                                flush_interval_secs: 3600,
                                max_size_bytes: u64::MAX,
                                compression_level: 3,
                                cipher: None,
                            },
                        )
                        .unwrap();
                        for i in 0..(n_segments * 4) as u64 {
                            buf.append(Item {
                                id: i,
                                payload: format!("payload-{i}"),
                            })
                            .unwrap();
                        }
                    }
                    tmp
                },
                |tmp| {
                    let buf = SegmentBuffer::<Item>::open(
                        tmp.path(),
                        SegmentConfig {
                            max_batch_events: 4,
                            flush_interval_secs: 3600,
                            max_size_bytes: u64::MAX,
                            compression_level: 3,
                            cipher: None,
                        },
                    )
                    .unwrap();
                    black_box(buf.pending_count());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recover);
criterion_main!(benches);
