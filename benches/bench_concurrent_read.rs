//! Benchmark: concurrent `read_from` — multiple reader threads reading
//! simultaneously from the same buffer.
//!
//! Measures how read throughput scales with reader thread count. All readers
//! read the same in-memory data (no disk I/O).
//!
//! Run with:
//!   cargo bench --bench bench_concurrent_read --features encryption

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
use std::sync::Arc;
use std::thread;
#[path = "support.rs"]
mod support;

fn bench_concurrent_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_read");

    let n = 10_000usize;

    for &n_threads in &[1usize, 2, 4, 8] {
        // Setup: 10k items in memory, shared across reader threads.
        let (buf, _tmp) = support::open_buffer(usize::MAX);
        for i in 0..n as u64 {
            buf.append(support::item(i)).unwrap();
        }
        let buf = Arc::new(buf);

        group.bench_function(format!("read_{n_threads}_threads"), |b| {
            b.iter(|| {
                let mut handles = Vec::new();
                for _ in 0..n_threads {
                    let buf = buf.clone();
                    handles.push(thread::spawn(move || {
                        let items = buf.read_from(0, n).unwrap();
                        black_box(items.len());
                    }));
                }
                for h in handles {
                    h.join().unwrap();
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_concurrent_read);
criterion_main!(benches);
