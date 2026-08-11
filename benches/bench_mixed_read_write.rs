//! Benchmark: mixed read/write — producer + consumer threads concurrently.
//!
//! Models the canonical cloud-sync workload: one producer thread appending
//! items while one consumer thread reads + deletes them.
//!
//! Run with:
//!   cargo bench --bench bench_mixed_read_write --features encryption

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
use std::sync::Arc;
use std::thread;
#[path = "support.rs"]
mod support;

fn bench_mixed_read_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_read_write");

    let n_items = 10_000usize;

    for &n_producers in &[1usize, 2, 4] {
        // 1 consumer per run, varying producer count.
        group.bench_function(format!("{n_producers}_producer_1_consumer"), |b| {
            b.iter_batched_ref(
                || {
                    let (buf, tmp) = support::open_buffer(usize::MAX);
                    (Arc::new(buf), tmp)
                },
                |(buf, _tmp)| {
                    let items_per_producer = n_items / n_producers;

                    let mut handles = Vec::new();

                    // Consumer: read + delete in a loop.
                    let buf_c = buf.clone();
                    handles.push(thread::spawn(move || {
                        let mut total_read = 0usize;
                        while total_read < n_items {
                            if let Ok(items) = buf_c.read_from(0, n_items) {
                                total_read += items.len();
                                if !items.is_empty() {
                                    let last_seq = total_read as u64 - 1;
                                    let _ = buf_c.delete_acked(last_seq);
                                }
                            }
                        }
                        black_box(total_read);
                    }));

                    // Producers: append items.
                    for p in 0..n_producers {
                        let buf_p = buf.clone();
                        let base = (p * items_per_producer) as u64;
                        handles.push(thread::spawn(move || {
                            for i in 0..items_per_producer as u64 {
                                buf_p.append(support::item(base + i)).unwrap();
                            }
                        }));
                    }

                    for h in handles {
                        h.join().unwrap();
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_mixed_read_write);
criterion_main!(benches);
