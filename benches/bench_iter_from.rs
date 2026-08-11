//! Benchmark: `iter_from` (materialising iterator) vs `for_each_from`
//! (lending callback) vs `read_from` (owned Vec).
//!
//! Quantifies the tradeoff between the three read APIs at different scales.
//!
//! Run with:
//!   cargo bench --bench bench_iter_from --features encryption

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

fn bench_iter_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("iter_vs_foreach_vs_read");

    for &n in &[1_000usize, 10_000] {
        // Setup: N unflushed items in memory, shared read-only across all
        // three benches. The buffer state is constant (read-only operations).
        let (buf, _tmp) = support::open_buffer(usize::MAX);
        for i in 0..n as u64 {
            buf.append(support::item(i)).unwrap();
        }

        group.bench_function(format!("read_from/{n}"), |b| {
            b.iter(|| {
                let items = buf.read_from(0, n).unwrap();
                black_box(items.len());
            });
        });

        group.bench_function(format!("for_each_from/{n}"), |b| {
            b.iter(|| {
                let count = buf.for_each_from(0, n, |_, _| {}).unwrap();
                black_box(count);
            });
        });

        group.bench_function(format!("iter_from/{n}"), |b| {
            b.iter(|| {
                let iter = buf.iter_from(0, n).unwrap();
                let mut count = 0usize;
                for (seq, item) in iter {
                    black_box((seq, &item));
                    count += 1;
                }
                black_box(count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_iter_from);
criterion_main!(benches);
