//! Benchmark: concurrent append throughput (MPMC under mutex contention).
//!
//! The crate's value proposition is multi-producer support via a
//! `parking_lot::Mutex`. All other benches are single-threaded. This benchmark
//! measures how aggregate throughput degrades as writer threads compete for
//! the mutex, and how `append_all` (one lock acquisition per batch) compares
//! to `append` (one lock acquisition per item) under contention.
//!
//! Each thread appends `ITEMS_PER_THREAD` items, then the main thread flushes
//! once. The throughput is total items / wall time — the mutex contention is
//! the variable under test.
//!
//! Run with: `cargo bench --bench bench_concurrent_append`

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

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::hint::black_box;
use std::sync::Arc;
use std::thread;

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

const ITEMS_PER_THREAD: usize = 10_000;

fn make_items(offset: u64, count: usize) -> Vec<Item> {
    (0..count)
        .map(|i| Item {
            id: offset + i as u64,
            payload: format!("payload-{}", offset + i as u64),
        })
        .collect()
}

fn bench_concurrent_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_append");
    let total_items = (ITEMS_PER_THREAD * 4) as u64; // 4 threads max
    group.throughput(Throughput::Elements(total_items));

    for &n_threads in &[1usize, 2, 4, 8] {
        let total = (ITEMS_PER_THREAD * n_threads) as u64;
        group.throughput(Throughput::Elements(total));

        // --- append (one lock per item) ---
        group.bench_function(format!("append_{n_threads}_threads"), |b| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    let cfg = SegmentConfig::builder()
                        .flush_policy(FlushPolicy::Manual)
                        .max_size_bytes(u64::MAX)
                        .compression_level(3)
                        .build();
                    let buf = Arc::new(SegmentBuffer::<Item>::open(tmp.path(), cfg).unwrap());
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    let barrier = Arc::new(std::sync::Barrier::new(n_threads + 1));
                    let mut handles = Vec::with_capacity(n_threads);
                    for t in 0..n_threads {
                        let buf = buf.clone();
                        let barrier = barrier.clone();
                        let offset = (t * ITEMS_PER_THREAD) as u64;
                        handles.push(thread::spawn(move || {
                            barrier.wait();
                            for i in 0..ITEMS_PER_THREAD as u64 {
                                buf.append(Item {
                    id: offset + i,
                    payload: format!("payload-{}", offset + i),
                }).unwrap();
                            }
                        }));
                    }
                    barrier.wait(); // release all threads simultaneously
                    for h in handles {
                        h.join().unwrap();
                    }
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            );
        });

        // --- append_all (one lock per batch) ---
        group.bench_function(format!("append_all_{n_threads}_threads"), |b| {
            b.iter_with_setup(
                || {
                    let tmp = tempfile::tempdir().unwrap();
                    let cfg = SegmentConfig::builder()
                        .flush_policy(FlushPolicy::Manual)
                        .max_size_bytes(u64::MAX)
                        .compression_level(3)
                        .build();
                    let buf = Arc::new(SegmentBuffer::<Item>::open(tmp.path(), cfg).unwrap());
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    let barrier = Arc::new(std::sync::Barrier::new(n_threads + 1));
                    let mut handles = Vec::with_capacity(n_threads);
                    for t in 0..n_threads {
                        let buf = buf.clone();
                        let barrier = barrier.clone();
                        let offset = (t * ITEMS_PER_THREAD) as u64;
                        let items = make_items(offset, ITEMS_PER_THREAD);
                        handles.push(thread::spawn(move || {
                            barrier.wait();
                            buf.append_all(items).unwrap();
                        }));
                    }
                    barrier.wait();
                    for h in handles {
                        h.join().unwrap();
                    }
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_concurrent_append);
criterion_main!(benches);
