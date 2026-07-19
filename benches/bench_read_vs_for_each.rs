//! Benchmark: `read_from` (clones every item into a Vec<T>) vs `for_each_from`
//! (lending iterator; zero clones for in-memory pending items).
//!
//! Quantifies the clone cost the lending iterator was introduced to avoid.
//!
//! Run with:
//!   cargo bench --bench bench_read_vsForEach --features encryption

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

fn bench_read_vs_for_each(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_vs_for_each");

    for &n in &[1_000usize, 10_000] {
        // Both benches share the same buffer state: N unflushed items in
        // memory (no segment files), so the clone-vs-borrow delta is the
        // entire signal. Use Manual flush policy to keep items in memory.
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
