//! Benchmark: `append_all` batch primitive vs a loop of `append` calls.
//!
//! `append_all` acquires the mutex once for the whole batch; the loop pays
//! the lock-acquisition cost per item. This bench quantifies the delta so
//! callers can decide which API fits their workload.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

fn bench_append_all_vs_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_all");
    for batch_size in [100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(batch_size as u64));

        group.bench_function(format!("loop_append_{batch_size}"), |b| {
            b.iter_with_setup(
                || support::open_buffer(100_000), // don't auto-flush mid-bench
                |(buf, _tmp)| {
                    for i in 0..batch_size as u64 {
                        buf.append(support::item(i)).unwrap();
                    }
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            )
        });

        group.bench_function(format!("append_all_{batch_size}"), |b| {
            b.iter_with_setup(
                || support::open_buffer(100_000),
                |(buf, _tmp)| {
                    let items: Vec<_> = (0..batch_size as u64).map(support::item).collect();
                    buf.append_all(items).unwrap();
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_append_all_vs_loop);
criterion_main!(benches);
