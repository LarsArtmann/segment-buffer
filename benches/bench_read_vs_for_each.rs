//! Benchmark: `read_from` (clones every item into a Vec<T>) vs `for_each_from`
//! (lending iterator; zero clones for in-memory pending items).
//!
//! Quantifies the clone cost the lending iterator was introduced to avoid.
//!
//! Run with:
//!   cargo bench --bench bench_read_vs_for_each --features encryption
//!
//! # Why the shared buffer is correct
//!
//! Both `read_from` and `for_each_from` are read-only — they do not modify the
//! buffer's `unflushed` Vec. The buffer has exactly N items at setup time, and
//! every iteration reads those same N items. The buffer state is constant
//! across iterations, so there is no state-growth bias. The shared buffer
//! stays hot in L1/L2 cache across iterations, which mirrors the production
//! "hot buffer" case (the common workload). Per-iteration setup via
//! `iter_batched_ref` would conflate the N append costs into the read
//! measurement, which is wrong for a read-only bench.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

fn bench_read_vs_for_each(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_vs_for_each");

    for &n in &[1_000usize, 10_000] {
        // Both benches share the same buffer state: N unflushed items in
        // memory (no segment files), so the clone-vs-borrow delta is the
        // entire signal. usize::MAX batch size keeps items in memory.
        //
        // Shared buffer is correct because both operations are READ-ONLY —
        // they do not modify `unflushed`. The buffer state is constant across
        // iterations, mirroring the production "hot buffer" case.
        let (buf, _tmp) = support::open_buffer(usize::MAX);
        for i in 0..n as u64 {
            buf.append(support::item(i)).unwrap();
        }

        group.bench_function(format!("read_from/{n}"), |b| {
            b.iter(|| {
                let items = buf.read_from(0, n).unwrap();
                black_box(items.len());
            })
        });

        group.bench_function(format!("for_each_from/{n}"), |b| {
            b.iter(|| {
                let count = buf.for_each_from(0, n, |_, _| {}).unwrap();
                black_box(count);
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_read_vs_for_each);
criterion_main!(benches);
