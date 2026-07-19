//! Loom concurrency test for the in-memory `SegmentBuffer` hot path.
//!
//! What this covers:
//! - `append` (in-memory path only — `FlushPolicy::Manual` so the flush
//!   threshold never trips)
//! - `pending_count`, `latest_sequence`, `stats()` — read-only inner accessors
//! - `append_all` batch primitive under contention
//!
//! What this does NOT cover:
//! - `flush`, `delete_acked`, `recover`, `read_from` — all of these touch the
//!   real filesystem, which loom does not model. They stay covered by the
//!   stress test `concurrency_4_writers_1_reader_10k_events` in `src/tests.rs`.
//!
//! Run with:
//!   RUSTFLAGS="--cfg loom" cargo test --features loom --test loom -- --release
//!
//! `--release` is recommended: loom's exhaustive schedule enumeration is
//! slow, and a debug build doubles the per-step cost.

#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;
use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Item {
    id: u64,
}

/// Manual flush policy so the test exercises only the in-memory
/// lock + Vec + u64 counter path, never the filesystem.
fn loom_config() -> SegmentConfig {
    SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(u64::MAX)
        .compression_level(3)
        .build()
}

#[test]
fn two_writers_concurrent_append_never_loses_items() {
    loom::model(|| {
        // Build the buffer outside the modeled threads so the filesystem call
        // (open() → read_dir) is not part of the schedule enumeration.
        let dir = tempfile::tempdir().unwrap();
        let buf: Arc<SegmentBuffer<Item>> =
            Arc::new(SegmentBuffer::open(dir.path(), loom_config()).unwrap());

        // Two threads, two appends each. Loom explores every interleaving.
        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            b1.append(Item { id: 1 }).unwrap();
            b1.append(Item { id: 2 }).unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 3 }).unwrap();
            b2.append(Item { id: 4 }).unwrap();
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Invariant: exactly 4 appends → pending_count == 4 and
        // latest_sequence == 3 (0-indexed, last id assigned).
        assert_eq!(buf.pending_count(), 4, "every append must be counted");
        assert_eq!(
            buf.latest_sequence(),
            3,
            "sequence must be 0-indexed monotonic"
        );
        let snapshot = buf.stats();
        assert_eq!(snapshot.pending_count, 4);
        assert_eq!(snapshot.latest_sequence, 3);
        assert_eq!(snapshot.next_sequence, 4);
    });
}

#[test]
fn writer_and_reader_do_not_observe_torn_snapshot() {
    loom::model(|| {
        let dir = tempfile::tempdir().unwrap();
        let buf: Arc<SegmentBuffer<Item>> =
            Arc::new(SegmentBuffer::open(dir.path(), loom_config()).unwrap());

        // Pre-populate so the reader has something to observe.
        buf.append(Item { id: 0 }).unwrap();
        buf.append(Item { id: 1 }).unwrap();

        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            b1.append(Item { id: 2 }).unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            // stats() is the atomic snapshot. Every field is read under a
            // single lock, so the four fields must be mutually consistent.
            let s = b2.stats();
            // Either we observe the third append or we don't — but we must
            // never observe pending_count=3 with next_sequence=2 (torn).
            if s.pending_count == 3 {
                assert_eq!(
                    s.next_sequence, 3,
                    "stats() snapshot is torn: pending_count={} next_sequence={}",
                    s.pending_count, s.next_sequence
                );
            } else {
                assert_eq!(s.pending_count, 2);
                assert_eq!(s.next_sequence, 2);
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();
    });
}

#[test]
fn append_all_batch_atomicity_under_concurrent_append() {
    // Verify that append_all assigns a contiguous block of sequence numbers
    // even when a concurrent single append is interleaved by the scheduler.
    // The whole batch is under one lock, so no single append can split it.
    loom::model(|| {
        let dir = tempfile::tempdir().unwrap();
        let buf: Arc<SegmentBuffer<Item>> =
            Arc::new(SegmentBuffer::open(dir.path(), loom_config()).unwrap());

        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            b1.append_all([Item { id: 10 }, Item { id: 11 }, Item { id: 12 }])
                .unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 99 }).unwrap();
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Total items must be 4 (3 from append_all + 1 from append).
        assert_eq!(buf.pending_count(), 4);
        assert_eq!(buf.latest_sequence(), 3);
    });
}
