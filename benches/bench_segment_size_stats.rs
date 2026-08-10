//! Benchmark: `segment_size_stats()` — the on-demand `O(n_segments)` scan
//! that walks every segment file via `store.segment_size`, sorts the sizes,
//! and computes count / min / max / mean / p50 / p90.
//!
//! Quantifies the scan cost at three directory sizes (100 / 1k / 10k segments)
//! so callers can decide whether the distribution query is cheap enough to
//! call inside a tuning loop or whether it should be batched.
//!
//! Run with:
//!   cargo bench --bench bench_segment_size_stats

// Non-production code: unwrap/expect on setup failures, `as` conversions for
// batch sizes, and counter arithmetic are idiomatic and safe here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

/// Items per segment — small batches so the directory has many small files,
/// mirroring a real tuning scenario where `segment_size_stats` is most useful.
const ITEMS_PER_SEGMENT: usize = 10;

fn bench_segment_size_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_size_stats");
    group.sample_size(20); // 10k segments is I/O-heavy; fewer samples keep it fast

    for &n_segments in &[100usize, 1_000, 10_000] {
        let (buf, _tmp) = support::open_buffer_with_segments(n_segments, ITEMS_PER_SEGMENT);

        group.bench_with_input(
            BenchmarkId::from_parameter(n_segments),
            &n_segments,
            |b, &_n| {
                b.iter(|| {
                    let stats = buf.segment_size_stats().unwrap();
                    black_box((
                        stats.count,
                        stats.min_bytes,
                        stats.max_bytes,
                        stats.mean_bytes,
                        stats.p50_bytes,
                        stats.p90_bytes,
                    ))
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_segment_size_stats);
criterion_main!(benches);
