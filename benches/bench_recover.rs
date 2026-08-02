//! Benchmark: crash recovery throughput — `SegmentBuffer::open()` on a
//! directory with pre-existing segment files.

// Non-production code: unwrap/expect on setup failures, `as` conversions for
// batch sizes, and counter arithmetic are idiomatic and safe here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery,
)]


use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
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
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recover);
criterion_main!(benches);
