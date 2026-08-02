//! Benchmark: `delete_acked` throughput after populating segments.

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

fn bench_delete_acked(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_acked");
    group.throughput(criterion::Throughput::Elements(100));
    group.bench_function("100_segments", |b| {
        b.iter_with_setup(
            || {
                let (buf, tmp) = support::open_buffer(4); // small → many segment files
                for i in 0..400 {
                    buf.append(support::item(i)).unwrap();
                }
                (buf, tmp)
            },
            |(buf, _tmp)| {
                // Ack the first 50 segments (200 events of 400).
                let deleted = buf.delete_acked(199).unwrap();
                black_box(deleted);
            },
        );
    });
    group.finish();
}

/// Scale test: ack across a 10,000-segment directory. With `batch_size = 1`,
/// 10,000 events produce 10,000 segment files; acking half of them exercises
/// the scan + stat + unlink loop at the scale monitor365 actually hits.
fn bench_delete_acked_10k_segments(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_acked");
    group.throughput(criterion::Throughput::Elements(10_000));
    group.sample_size(10); // each iteration is heavy; fewer samples keeps runtime sane
    group.bench_function("10k_segments", |b| {
        b.iter_with_setup(
            || {
                // batch_size = 1 → every append becomes its own segment file.
                let (buf, tmp) = support::open_buffer(1);
                for i in 0..10_000 {
                    buf.append(support::item(i)).unwrap();
                }
                (buf, tmp)
            },
            |(buf, _tmp)| {
                // Ack the first 5,000 segments.
                let deleted = buf.delete_acked(4_999).unwrap();
                black_box(deleted);
            },
        );
    });
    group.finish();
}

criterion_group!(benches, bench_delete_acked, bench_delete_acked_10k_segments);
criterion_main!(benches);
