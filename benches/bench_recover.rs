//! Benchmark: crash recovery throughput — `SegmentBuffer::open()` on a
//! directory with pre-existing segment files.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

use segment_buffer::SegmentBuffer;

fn bench_recover(c: &mut Criterion) {
    let mut group = c.benchmark_group("recover");
    for n_segments in [10usize, 100, 1_000] {
        group.throughput(criterion::Throughput::Elements(n_segments as u64));
        group.bench_function(format!("{n_segments}_segments"), |b| {
            b.iter_with_setup(
                || {
                    let (buf, tmp) = support::open_buffer(4);
                    for i in 0..(n_segments * 4) as u64 {
                        buf.append(support::item(i)).unwrap();
                    }
                    drop(buf); // flush all segments to disk, then drop the handle
                    tmp
                },
                |tmp| {
                    // Re-open the directory: this is the recovery path being measured.
                    let buf = SegmentBuffer::<support::Item>::open(tmp.path(), support::config(4))
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
