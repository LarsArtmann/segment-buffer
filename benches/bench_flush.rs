//! Benchmark: `flush()` — isolates the encode pipeline (CBOR + zstd + write).
//!
//! The append benchmarks include mutex + sequence-number overhead. This bench
//! measures the pure flush cost: items are pre-loaded in-memory with
//! `FlushPolicy::Manual`, then `flush()` is the only operation measured.
//!
//! Run with:
//!   cargo bench --bench bench_flush --features encryption

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

fn bench_flush(c: &mut Criterion) {
    let mut group = c.benchmark_group("flush");

    for &n in &[100usize, 1_000, 10_000] {
        group.bench_function(format!("flush_{n}"), |b| {
            b.iter_batched_ref(
                || {
                    let (buf, tmp) = support::open_buffer(usize::MAX);
                    for i in 0..n as u64 {
                        buf.append(support::item(i)).unwrap();
                    }
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    buf.flush().unwrap();
                    let _ = black_box(buf.stats());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_flush);
criterion_main!(benches);
