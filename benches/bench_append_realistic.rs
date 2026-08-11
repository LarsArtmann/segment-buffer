//! Benchmark: `append` with realistic payloads (text, JSON) vs the uniform
//! baseline. Quantifies how payload entropy affects throughput.
//!
//! Run with:
//!   cargo bench --bench bench_append_realistic --features encryption

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

fn bench_append_realistic(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_realistic");

    for &batch in &[100usize, 1_000, 10_000] {
        // Uniform (baseline) — highly compressible.
        group.bench_function(format!("uniform/{batch}"), |b| {
            b.iter_batched_ref(
                || support::open_buffer(usize::MAX),
                |(buf, _tmp)| {
                    for i in 0..batch as u64 {
                        buf.append(support::item(i)).unwrap();
                    }
                    let _ = black_box(buf.stats());
                },
                BatchSize::SmallInput,
            );
        });

        // Text — realistic English-prose-shaped payloads.
        group.bench_function(format!("text/{batch}"), |b| {
            b.iter_batched_ref(
                || support::open_buffer(usize::MAX),
                |(buf, _tmp)| {
                    for i in 0..batch as u64 {
                        buf.append(support::text_item(i)).unwrap();
                    }
                    let _ = black_box(buf.stats());
                },
                BatchSize::SmallInput,
            );
        });

        // JSON — structured JSON objects.
        group.bench_function(format!("json/{batch}"), |b| {
            b.iter_batched_ref(
                || support::open_buffer(usize::MAX),
                |(buf, _tmp)| {
                    for i in 0..batch as u64 {
                        buf.append(support::json_item(i)).unwrap();
                    }
                    let _ = black_box(buf.stats());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_append_realistic);
criterion_main!(benches);
