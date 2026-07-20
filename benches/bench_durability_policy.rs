//! Benchmark: append + flush A/B across the three `DurabilityPolicy` variants.
//!
//! Measures the per-flush fsync cost the policies trade between. Typical
//! result on a Linux host with nvme + ext4:
//!
//! - `Maximal`:  baseline + 1× `file.sync_all()` + 1× `dir.sync_all()`
//!   per flush. The dir fsync is the heaviest single syscall on the path.
//! - `Segment`:  baseline + 1× `file.sync_all()` per flush (today's
//!   pre-v0.5.0 default).
//! - `Throughput`: no fsync at all — the kernel's dirty-page flusher
//!   handles disk sync on its own schedule. This is the cloud-sync
//!   recommended default and is typically the fastest by a wide margin.
//!
//! Run with: `cargo bench --bench bench_durability_policy`

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
use std::hint::black_box;
#[path = "support.rs"]
mod support;

/// Open a buffer with a specific `DurabilityPolicy`. Manual flush so each
/// iteration pays exactly one flush cost (vs being amortised across auto-flush).
fn open_with_policy(policy: DurabilityPolicy) -> (SegmentBuffer<support::Item>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(u64::MAX)
        .compression_level(3)
        .durability(policy)
        .build();
    let buf = SegmentBuffer::open(tmp.path(), cfg).unwrap();
    (buf, tmp)
}

fn bench_durability_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("durability_policy");
    let batch_size = 1000usize;
    group.throughput(Throughput::Elements(batch_size as u64));

    for policy in [
        DurabilityPolicy::Maximal,
        DurabilityPolicy::Segment,
        DurabilityPolicy::Throughput,
    ] {
        let label = format!("flush_batch_{batch_size}/{policy:?}");
        group.bench_function(label, |b| {
            b.iter_with_setup(
                || open_with_policy(policy),
                |(buf, _tmp)| {
                    for i in 0..batch_size as u64 {
                        buf.append(support::item(i)).unwrap();
                    }
                    // The single flush — this is where the policy
                    // differences land (file.sync_all + dir.sync_all vs
                    // neither).
                    buf.flush().unwrap();
                    black_box(buf.latest_sequence());
                },
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_durability_policy);
criterion_main!(benches);
