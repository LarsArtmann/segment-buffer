//! Standalone hot-path driver for flamegraph profiling.
//!
//! Runs the `append + flush` cycle in a tight loop so the sampler spends its
//! budget on the segment-buffer code path rather than criterion's measurement
//! harness. Built with frame pointers + debug symbols, then sampled with
//! `perf record` and folded into a flamegraph.
//!
//! See docs/perf/2026-07-20_hot-path-flamegraph.md for methodology and
//! analysis.

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use tempfile::tempdir;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Item {
    id: u64,
    payload: String,
}

fn main() {
    let tmp = tempdir().expect("tempdir");
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(u64::MAX)
        .compression_level(3)
        .build();
    let buf: SegmentBuffer<Item> = SegmentBuffer::open(tmp.path(), config).expect("open");

    // Two phases:
    //  1. batch_1 hot path: one item per flush (worst case for per-write overhead)
    //  2. batch_1000 hot path: 1000 items per flush (amortized case)
    //
    // We run both back-to-back so the flamegraph shows both regimes; the
    // batch_1 regime is where the v0.1.0 → v0.2.0 regression lived.

    let n = std::hint::black_box(200_000);

    // Phase 1: batch_1 (append + flush per item).
    for i in 0..n {
        let item = Item {
            id: i,
            payload: format!("payload-{i}"),
        };
        buf.append(item).expect("append");
        buf.flush().expect("flush");
    }

    // Phase 2: batch_1000 (append 1000, flush once), 200 batches.
    for batch in 0..200 {
        let base = batch * 1000;
        for i in 0..1000u64 {
            let item = Item {
                id: base + i,
                payload: format!("payload-{base}-{i}"),
            };
            buf.append(item).expect("append");
        }
        buf.flush().expect("flush");
    }

    // Sink: prevent the compiler from eliding everything.
    eprintln!("final seq: {}", buf.latest_sequence());
}
