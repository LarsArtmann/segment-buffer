//! Benchmark: read_from throughput with varying limits.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

fn bench_read_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_from");
    for limit in [100usize, 1_000, 10_000] {
        group.throughput(criterion::Throughput::Elements(limit as u64));
        group.bench_function(format!("limit_{limit}"), |b| {
            b.iter_with_setup(
                || {
                    let (buf, tmp) = support::open_buffer(100_000);
                    for i in 0..10_000 {
                        buf.append(support::item(i)).unwrap();
                    }
                    buf.flush().unwrap();
                    (buf, tmp)
                },
                |(buf, _tmp)| {
                    let events = buf.read_from(0, limit).unwrap();
                    black_box(events.len());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_read_from);
criterion_main!(benches);
