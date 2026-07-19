//! Benchmark: append throughput at varying batch sizes.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("append");
    for batch_size in [1usize, 100, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(batch_size as u64));
        group.bench_function(format!("batch_{batch_size}"), |b| {
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
    }
    group.finish();
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
