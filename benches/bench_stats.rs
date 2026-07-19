//! Benchmark: `stats()` (single lock + snapshot) vs the equivalent information
//! gathered via individual accessors (each of which takes the mutex).
//!
//! The `stats()` doc comment on `SegmentBuffer` claims it is "cheaper and more
//! consistent than calling [`pending_count`](SegmentBuffer::pending_count),
//! [`latest_sequence`](SegmentBuffer::latest_sequence),
//! [`store_pressure`](SegmentBuffer::store_pressure) etc. individually." This
//! benchmark either backs that claim with numbers or surfaces that it does not,
//! so the doc comment can be corrected.
//!
//! Run with:
//!   cargo bench --bench bench_stats --features encryption

use criterion::{black_box, criterion_group, criterion_main, Criterion};
#[path = "support.rs"]
mod support;

fn bench_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("stats");

    // Build once outside the measured closure so we measure only the accessors.
    let (buf, _tmp) = support::open_buffer(100_000);
    for i in 0..1_000u64 {
        buf.append(support::item(i)).unwrap();
    }
    buf.flush().unwrap();

    group.bench_function("stats_snapshot", |b| {
        b.iter(|| {
            let snapshot = buf.stats();
            black_box((
                snapshot.pending_count,
                snapshot.latest_sequence,
                snapshot.next_sequence,
                snapshot.head_sequence,
                snapshot.approx_disk_bytes,
                snapshot.max_size_bytes,
                snapshot.store_pressure,
            ))
        })
    });

    group.bench_function("individual_accessors", |b| {
        b.iter(|| {
            let pending = buf.pending_count();
            let latest = buf.latest_sequence();
            let pressure = buf.store_pressure();
            black_box((pending, latest, pressure))
        })
    });

    group.finish();
}

criterion_group!(benches, bench_stats);
criterion_main!(benches);
