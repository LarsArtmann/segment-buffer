//! Benchmark: read_from throughput with varying limits.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

fn bench_read_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_from");
    for limit in [100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(limit as u64));
        group.bench_function(format!("limit_{limit}"), |b| {
            b.iter_with_setup(
                || {
                    let (buf, tmp) = support::open_buffer(100_000);
                    for i in 0..10_000 {
                        buf.append(support::item(i)).unwrap();
                    }
                    buf.flush().unwrap();
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

/// Measure the `scan_segments` cache that landed in v0.4.0.
///
/// The cache stores the result of `fs::read_dir` + `parse_filename` for every
/// segment file, invalidated by `flush` / `delete_acked` / `recover`. A cache
/// hit skips the directory scan but still opens + decompresses + deserialises
/// each segment file, so the win is bounded by `readdir + parse` cost.
///
/// - **Cold** path: `PerIteration` setup drops the cache before every timed
///   call, so every measurement pays the `readdir`.
/// - **Warm** path: setup pre-warms the cache with one untimed `read_from`,
///   then the timed call hits the cache.
///
/// The two paths are benchmarked across 10 / 100 / 1 000 segment files so the
/// scaling with directory size is visible.
fn bench_read_from_scan_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_from_scan_cache");
    let items_per_segment = 100;
    let limit = 100;

    for &n_segments in &[10usize, 100, 1_000] {
        group.throughput(criterion::Throughput::Elements(limit as u64));

        group.bench_function(format!("cold_{n_segments}_segments"), |b| {
            b.iter_batched(
                || support::open_buffer_with_segments(n_segments, items_per_segment),
                |(buf, _tmp)| {
                    let events = buf.read_from(0, limit).unwrap();
                    black_box(events.len());
                },
                BatchSize::PerIteration,
            )
        });

        group.bench_function(format!("warm_{n_segments}_segments"), |b| {
            b.iter_batched(
                || {
                    let (buf, tmp) =
                        support::open_buffer_with_segments(n_segments, items_per_segment);
                    // Prime the scan cache with an untimed first call.
                    let _ = buf.read_from(0, limit).unwrap();
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    let events = buf.read_from(0, limit).unwrap();
                    black_box(events.len());
                },
                BatchSize::PerIteration,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_read_from, bench_read_from_scan_cache);
criterion_main!(benches);
