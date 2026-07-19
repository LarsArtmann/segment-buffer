//! Benchmark: delete_acked throughput after populating segments.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

fn bench_delete_acked(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_acked");
    group.throughput(criterion::Throughput::Elements(100));
    group.bench_function("100_segments", |b| {
        b.iter_with_setup(
            || {
                let (buf, tmp) = support::open_buffer(4); // small → many segment files
                for i in 0..400 {
                    buf.append(support::item(i)).unwrap();
                }
                (buf, tmp)
            },
            |(buf, _tmp)| {
                // Ack the first 50 segments (200 events of 400).
                let deleted = buf.delete_acked(199).unwrap();
                black_box(deleted);
            },
        )
    });
    group.finish();
}

criterion_group!(benches, bench_delete_acked);
criterion_main!(benches);
