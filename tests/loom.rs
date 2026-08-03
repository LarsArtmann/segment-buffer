//! Loom concurrency tests for `SegmentBuffer`.
//!
//! ## What this covers
//!
//! 1. **In-memory hot path** (`append`, `pending_count`, `latest_sequence`,
//!    `stats`, `append_all`) — original coverage, all under
//!    `FlushPolicy::Manual` so the flush threshold never trips.
//! 2. **`delete_acked` + `append` interleaving** (the v0.5.0 expansion) —
//!    exhaustively enumerated via a [`MockStore`] backed by
//!    `loom::sync::Mutex<HashMap<..>>`. This is the segment of the
//!    concurrency surface that was previously only covered *statistically*
//!    by the stress test `concurrency_4_writers_1_reader_10k_events`. Loom
//!    now covers it *exhaustively*: every interleaving of two threads with
//!    a handful of sync ops is explored.
//!
//! ## What this does NOT cover
//!
//! `flush` (other than the setup phase) and `recover` still touch byte-level
//! encode/decode that loom has no interest in enumerating. Their concurrency
//! contracts are exercised by the stress test in `src/tests.rs`.
//!
//! Note: `read_from` IS now covered — the two scan-cache tests below exercise
//! the cache-populate path (the `scan_segments` method) racing with
//! `flush`/`delete_acked`. These go through the full `read_from` pipeline
//! (scan → `read_bytes` → CBOR decode → zstd decompress → `read_segment`) on
//! the `MockStore`, which stores pre-encoded bytes so the pipeline exercises
//! real decode logic.
//!
//! ## The `MockStore` fidelity contract
//!
//! The mock models exactly the filesystem semantics the
//! `delete_acked` + `append` invariant depends on:
//!
//! - **Write atomicity:** [`SegmentStore::write_atomic`] is a single lock
//!   acquisition + insert. A concurrent reader either sees the previous
//!   value or the new one, never a partial write.
//! - **Remove idempotency:** [`SegmentStore::remove_segment`] returns `true`
//!   when this call removed the segment, `false` when it was already gone.
//!   Two concurrent `delete_acked` calls targeting the same segment do not
//!   double-count and do not error.
//! - **Scan semantics:** [`SegmentStore::scan`] returns the current keys
//!   sorted by `start`. The mock does not model "stale directory reads"
//!   (real FS doesn't have those either).
//! - **Sizing:** [`SegmentStore::segment_size`] returns the byte length of
//!   the stored payload. Missing segments return `0`.
//!
//! What the mock deliberately does NOT model: disk-full, permission errors,
//! filesystem corruption, partial writes from kernel crashes. These are not
//! concurrency properties — they are durability properties, covered by the
//! real-FS tests in `src/tests.rs`.
//!
//! ## Run command
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --features loom --test loom -- --release
//! ```
//!
//! `--release` is recommended: loom's exhaustive schedule enumeration is
//! slow, and a debug build doubles the per-step cost.

#![cfg(loom)]

use std::collections::HashMap;

use loom::sync::{Arc, Mutex};
use loom::thread;
use segment_buffer::{
    DurabilityPolicy, FlushPolicy, Result, SegmentBuffer, SegmentConfig, SegmentRange, SegmentStore,
};
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

// ---------------------------------------------------------------------------
// MockStore — the loom-aware I/O stub
// ---------------------------------------------------------------------------

/// Loom-aware in-memory replacement for [`segment_buffer::RealStore`].
///
/// Each method is a single `loom::sync::Mutex` acquisition over a
/// `HashMap<SegmentRange, Vec<u8>>`. Because loom treats each lock
/// acquisition as a schedule point, the mock faithfully models the
/// atomicity boundaries of the real filesystem operations:
/// `write_atomic` is atomic because it is one lock + one insert;
/// `remove_segment` returns whether this call removed the file (so
/// concurrent deletes do not double-count); `scan` returns a snapshot
/// of the current keys.
///
/// See the module doc for the full fidelity contract.
#[derive(Debug)]
struct MockStore {
    files: Mutex<HashMap<SegmentRange, Vec<u8>>>,
}

impl MockStore {
    fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
        }
    }
}

impl SegmentStore for MockStore {
    fn create_dir_all(&self) -> Result<()> {
        // The mock has no directory; create_dir_all is a no-op. The buffer
        // calls this during `open_with_store` before recovery runs.
        Ok(())
    }

    fn scan(&self) -> Result<Vec<SegmentRange>> {
        let files = self.files.lock().unwrap();
        let mut ranges: Vec<SegmentRange> = files.keys().copied().collect();
        ranges.sort_by_key(|r| r.start);
        Ok(ranges)
    }

    fn clean_tmp(&self) -> Result<usize> {
        // The mock never produces `.tmp` debris (write_atomic is a single
        // atomic insert), so there is nothing to clean.
        Ok(0)
    }

    fn segment_size(&self, range: SegmentRange) -> u64 {
        self.files
            .lock()
            .unwrap()
            .get(&range)
            .map(|v| v.len() as u64)
            .unwrap_or(0)
    }

    fn remove_segment(&self, range: SegmentRange) -> Result<bool> {
        // Single lock acquisition = atomic. Returns whether THIS call
        // removed the segment, so concurrent delete_acked calls do not
        // double-count.
        Ok(self.files.lock().unwrap().remove(&range).is_some())
    }

    fn write_atomic(
        &self,
        range: SegmentRange,
        payload: &[u8],
        _policy: DurabilityPolicy,
    ) -> Result<u64> {
        // Single lock acquisition = atomic. A concurrent reader observes
        // either the previous content or the new content, never a partial
        // write. The durability policy is ignored: the mock models
        // atomicity, not fsync behavior (loom does not model the disk).
        let len = payload.len() as u64;
        self.files.lock().unwrap().insert(range, payload.to_vec());
        Ok(len)
    }

    fn read_bytes(&self, range: SegmentRange) -> Result<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(&range)
            .cloned()
            .ok_or_else(|| {
                // NotFound is the only error path; mirrors RealStore::read_bytes
                // which does fs::read (returning NotFound for a missing file).
                std::io::Error::from(std::io::ErrorKind::NotFound).into()
            })
    }
}

// ---------------------------------------------------------------------------
// Sanity test — MockStore roundtrip (no loom schedule enumeration)
// ---------------------------------------------------------------------------

/// Smoke test for `MockStore` inside a (single-threaded) `loom::model`.
/// Verifies the basic write → scan → read → remove roundtrip works with
/// the expected semantics. Without this, a bug in the mock (e.g. scan
/// returning the wrong order, or write silently failing) would cause loom
/// to enumerate meaningless schedules and the actual `delete_acked +
/// append` proof would be vacuous. Wrapped in `loom::model` because
/// loom forbids touching its primitives outside one.
#[test]
fn mock_store_write_scan_read_remove_roundtrip() {
    loom::model(|| {
        let store = MockStore::new();
        let range = SegmentRange { start: 0, end: 3 };

        // Initially absent.
        assert_eq!(store.segment_size(range), 0);
        assert!(store.scan().unwrap().is_empty());
        assert!(store.read_bytes(range).is_err());

        // Write atomic.
        let payload = b"hello world";
        let written = store
            .write_atomic(range, payload, DurabilityPolicy::Segment)
            .unwrap();
        assert_eq!(written, payload.len() as u64);

        // Now visible.
        assert_eq!(store.segment_size(range), payload.len() as u64);
        let scanned = store.scan().unwrap();
        assert_eq!(scanned, vec![range]);
        assert_eq!(store.read_bytes(range).unwrap(), payload);

        // Remove returns true on the first call, false on the second.
        assert!(store.remove_segment(range).unwrap());
        assert!(!store.remove_segment(range).unwrap());

        // Now absent again.
        assert_eq!(store.segment_size(range), 0);
        assert!(store.scan().unwrap().is_empty());
    });
}

/// Scan ordering: MockStore must return segments sorted by `start`, so
/// `delete_acked`'s `new_head` computation (which keys off the first
/// not-deleted segment) sees the correct oldest survivor. Inside
/// `loom::model` for the same reason as the roundtrip test above.
#[test]
fn mock_store_scan_returns_segments_sorted_by_start() {
    loom::model(|| {
        let store = MockStore::new();
        // Insert out of order.
        store
            .write_atomic(
                SegmentRange { start: 10, end: 19 },
                b"b",
                DurabilityPolicy::Segment,
            )
            .unwrap();
        store
            .write_atomic(
                SegmentRange { start: 0, end: 9 },
                b"a",
                DurabilityPolicy::Segment,
            )
            .unwrap();
        store
            .write_atomic(
                SegmentRange { start: 20, end: 29 },
                b"c",
                DurabilityPolicy::Segment,
            )
            .unwrap();

        let scanned = store.scan().unwrap();
        assert_eq!(
            scanned,
            vec![
                SegmentRange { start: 0, end: 9 },
                SegmentRange { start: 10, end: 19 },
                SegmentRange { start: 20, end: 29 },
            ]
        );
    });
}

// ===========================================================================
// Original in-memory hot-path tests (unchanged from pre-v0.5.0 loom suite)
// ===========================================================================

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

// ===========================================================================
// delete_acked + append interleaving — the v0.5.0 coverage expansion
// ===========================================================================
//
// These tests are the deliverable of the SegmentStore trait refactor. They
// exhaustively enumerate every interleaving of `delete_acked` (which takes
// the inner mutex twice: once for scan setup, once for the head_seq clamp)
// and `append` (which takes the inner mutex once). The invariant under
// proof:
//
//     head_seq <= pending_start
//
// where `pending_start = next_seq - unflushed.len()` is the sequence
// number of the oldest still-unflushed item. If this is ever violated,
// `pending_count` under-reports the real backlog — silent data loss in a
// durable queue. The clamp at the end of `delete_acked` is what enforces
// it; these tests prove the clamp holds across every schedule.

/// Helper: open a buffer backed by a [`MockStore`] and return it wrapped
/// in a `loom::sync::Arc` so it can be shared across modeled threads.
///
/// Note on Arc types: the buffer's `open_with_store` takes the store as a
/// `std::sync::Arc<dyn SegmentStore + Send + Sync>` (the buffer's field
/// type uses std's Arc unconditionally — only the buffer's *mutex* swaps
/// to loom's under `--cfg loom`, not the store's reference-count). The
/// returned buffer, by contrast, is wrapped in `loom::sync::Arc` so the
/// ref-count itself is part of loom's schedule enumeration.
fn open_with_mock(config: SegmentConfig) -> Arc<SegmentBuffer<Item>> {
    let dir = tempfile::tempdir().unwrap();
    let store: std::sync::Arc<dyn SegmentStore + Send + Sync> =
        std::sync::Arc::new(MockStore::new());
    let buf = SegmentBuffer::open_with_store(dir.path(), config, store)
        .expect("open_with_store must succeed on a fresh mock");
    Arc::new(buf)
}

/// The headline invariant: `delete_acked` racing `append` must never let
/// `head_seq` advance past the in-memory pending window.
///
/// Setup: append 4 items, flush (one segment [0..=3] in the mock), then
/// append one more (so `unflushed == [item4]` and `pending_start == 4`).
/// Thread A: `delete_acked(3)` (acks the flushed segment). Thread B:
/// `append(item5)`.
///
/// In every interleaving, the post-state must satisfy
/// `pending_count >= 1` (the original `item4` is still there) — i.e.
/// `head_seq <= next_seq - 1`. Without the clamp, `head_seq` could
/// advance to `next_seq` (5 or 6 depending on schedule), under-reporting
/// the backlog by one item.
#[test]
fn delete_acked_during_append_never_loses_head() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        // Pre-populate: 4 items flushed as segment [0..=3].
        for i in 0..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();
        // One more in the in-memory pending window (item4 → seq 4).
        buf.append(Item { id: 4 }).unwrap();

        // Snapshot the minimum backlog we must observe at the end. item4
        // is in unflushed; item5 may or may not also be there at the end
        // depending on whether B ran, but it WILL run because we join.
        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            // Ack the flushed segment [0..=3].
            b1.delete_acked(3).unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 5 }).unwrap();
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Invariant: pending_count must include BOTH unflushed items
        // (item4 from setup, item5 from thread B). If head_seq advanced
        // past item4's seq, pending_count would under-report.
        let s = buf.stats();
        assert!(
            s.pending_count >= 2,
            "pending_count under-reported: {}, expected >= 2 (item4 + item5)",
            s.pending_count
        );
        // Self-consistency: stats() snapshot must not be torn.
        assert_eq!(
            s.pending_count,
            s.next_sequence.saturating_sub(s.head_sequence),
            "stats() snapshot is torn"
        );
    });
}

/// `delete_acked` acking past the flush boundary into the pending window
/// must still clamp `head_seq` to `pending_start`.
///
/// Same setup as
/// [`delete_acked_during_append_never_loses_head`], but the ack value (5)
/// covers BOTH the flushed segment [0..=3] AND the in-memory pending item
/// (seq 4). Without the clamp, `head_seq` would advance to 5 (or 6 with
/// the concurrent append), silently dropping item4 from the backlog.
#[test]
fn delete_acked_past_flush_boundary_with_concurrent_append() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        for i in 0..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();
        buf.append(Item { id: 4 }).unwrap();

        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            // Ack everything up to seq 5 (covers flushed segment AND
            // pending item). The clamp must still keep head_seq at
            // pending_start.
            b1.delete_acked(5).unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 5 }).unwrap();
        });

        h1.join().unwrap();
        h2.join().unwrap();

        let s = buf.stats();
        assert!(
            s.pending_count >= 2,
            "pending_count under-reported: {}, expected >= 2 (item4 + item5)",
            s.pending_count
        );
        assert_eq!(
            s.pending_count,
            s.next_sequence.saturating_sub(s.head_sequence),
            "stats() snapshot is torn"
        );
    });
}

/// `stats()` called concurrently with `delete_acked` + `append` must
/// return a self-consistent snapshot in every schedule. The single-lock
/// snapshot design guarantees `pending_count == next_seq - head_seq`.
#[test]
fn stats_snapshot_consistent_under_delete_plus_append() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        for i in 0..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();
        buf.append(Item { id: 4 }).unwrap();

        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            b1.delete_acked(3).unwrap();
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 5 }).unwrap();
        });
        let b3 = buf.clone();
        let h3 = thread::spawn(move || {
            // The snapshot must be self-consistent regardless of when
            // this thread observes the buffer state.
            let s = b3.stats();
            assert_eq!(
                s.pending_count,
                s.next_sequence.saturating_sub(s.head_sequence),
                "stats() snapshot is torn: pending_count={} next={} head={}",
                s.pending_count,
                s.next_sequence,
                s.head_sequence
            );
            // head_seq never exceeds next_seq (would imply negative backlog).
            assert!(
                s.head_sequence <= s.next_sequence,
                "head_seq {} exceeded next_seq {}",
                s.head_sequence,
                s.next_sequence
            );
        });

        h1.join().unwrap();
        h2.join().unwrap();
        h3.join().unwrap();
    });
}

/// Two concurrent `delete_acked` calls + an `append` must not panic, must
/// not double-count deletions, and must not corrupt the head_seq clamp.
/// The `remove_segment` trait method's idempotency contract is what makes
/// this safe — `RealStore` returns Ok(false) on NotFound, `MockStore`
/// returns Ok(false) when the HashMap key was already removed by the other
/// thread.
#[test]
fn delete_acked_idempotent_under_concurrent_append() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        for i in 0..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();
        buf.append(Item { id: 4 }).unwrap();

        let b1 = buf.clone();
        let h1 = thread::spawn(move || {
            // Two concurrent deleters targeting the same segment [0..=3].
            // Exactly one should report deleted=1; the other gets 0 (the
            // segment was already gone). The sum is at most 1.
            b1.delete_acked(3).unwrap()
        });
        let b2 = buf.clone();
        let h2 = thread::spawn(move || b2.delete_acked(3).unwrap());
        let b3 = buf.clone();
        let h3 = thread::spawn(move || b3.append(Item { id: 5 }).unwrap());

        let d1 = h1.join().unwrap();
        let d2 = h2.join().unwrap();
        h3.join().unwrap();

        // No double-count: the segment [0..=3] can only be removed once.
        assert!(
            d1 + d2 <= 1,
            "concurrent delete_acked double-counted: d1={d1} d2={d2} (sum should be <= 1)"
        );

        // Backlog invariant still holds across the three-way interleaving.
        let s = buf.stats();
        assert!(
            s.pending_count >= 2,
            "pending_count under-reported after concurrent delete+delete+append: {}",
            s.pending_count
        );
    });
}

// ===========================================================================
// scan_segments cache populate/invalidate interleaving — scan-cache coverage
// ===========================================================================
//
// These tests exercise `scan_segments` (the scan-cache populate path) under
// concurrent mutation. They are the first loom tests to exercise the read
// path (`read_from`) at all — the original 9 tests cover only the in-memory
// hot path (`append`/`stats`) and the `delete_acked` + `append` clamp.
//
// What is under proof:
//
// 1. **No deadlocks or panics** across every interleaving of `read_from`
//    (which calls `scan_segments` → `store.scan()` → cache publish) with
//    `flush` (which calls `store.write_atomic` → `invalidate_scan_cache`).
//
// 2. **Data integrity**: items returned by `read_from` are always a valid
//    subset of the true state — no phantom items, no duplicates, strictly
//    ascending.
//
// 3. **Eventual consistency**: after both threads settle, a subsequent
//    mutation (which invalidates the cache) followed by a `read_from`
//    returns the complete correct state.
//
// What is NOT under proof (and intentionally not asserted):
//
// The scan-cache **publication-overwrites-invalidation** race. If
// `scan_segments` publishes a stale segment list in the window between
// `flush`'s `write_atomic` and `invalidate_scan_cache`, the cache can
// serve stale data until the next mutation. This is the documented
// "transient gap" behavior — on real filesystems the mtime guard catches
// it; under the `MockStore` (which does not touch the real directory),
// `mtime_supported` is determined by the real tempdir and the guard may
// or may not fire. The test does not assert completeness during the race,
// only after settling.
//
// Tractability note: `read_from` goes through CBOR decode + zstd
// decompress (the MockStore faithfully stores and returns the encoded
// bytes from `write_atomic`). This adds computation per schedule but no
// extra loom sync points — the decode is pure computation. The sync
// surface is: `scan_cache` lock (×2), `MockStore` lock (scan + read_bytes),
// `decompressor` lock, `last_dir_mtime` lock, `inner` lock — ~8 per
// `read_from`, ~5 per `flush`. Total ~13 concurrent sync points across
// two threads, well within loom's practical enumeration range.

/// `read_from` under concurrent `flush` must never return corrupted,
/// duplicated, or out-of-order data. After the flusher settles, all
/// items must be visible.
///
/// Setup: flush segment [0..=3]. Thread A: `read_from(0, 100)`.
/// Thread B: `append(item4)` + `flush()`. Loom explores every
/// interleaving of the scan-cache populate (inside `read_from`'s
/// `scan_segments`) and the cache invalidation (inside `flush`).
///
/// After both threads join, append one more item and flush to
/// invalidate the cache, then verify all 6 items are visible —
/// proving the cache is eventually consistent regardless of the
/// race outcome.
#[test]
fn read_from_concurrent_flush_scan_cache_no_corruption() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        // Pre-populate: flush segment [0..=3].
        for i in 0..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();

        // Thread A: read_from — exercises scan_segments + read_segment.
        let b1 = buf.clone();
        let h1 = thread::spawn(move || b1.read_from(0, 100).unwrap());

        // Thread B: append + flush — exercises write_atomic + invalidate.
        let b2 = buf.clone();
        let h2 = thread::spawn(move || {
            b2.append(Item { id: 4 }).unwrap();
            b2.flush().unwrap();
        });

        let batch = h1.join().unwrap();
        h2.join().unwrap();

        // Invariant: every item in the batch is valid, strictly ascending,
        // and has no duplicates. The batch may be incomplete (transient gap
        // from the scan-cache publication race) but must never be wrong.
        let mut prev: Option<u64> = None;
        for item in &batch {
            assert!(
                item.id < 5,
                "phantom item in read_from result: id={} (valid range 0..=4)",
                item.id
            );
            if let Some(p) = prev {
                assert!(
                    item.id > p,
                    "out-of-order or duplicate item: {} after {}",
                    item.id,
                    p
                );
            }
            prev = Some(item.id);
        }

        // Eventual consistency: after settling, invalidate the cache (via
        // another flush) and verify all items are visible.
        buf.append(Item { id: 5 }).unwrap();
        buf.flush().unwrap();
        let all = buf.read_from(0, 100).unwrap();
        assert_eq!(
            all.len(),
            6,
            "all items must be visible after cache invalidation + re-scan"
        );
        for (i, item) in all.iter().enumerate() {
            assert_eq!(item.id, i as u64, "item at position {i} has wrong id");
        }
    });
}

/// `read_from` under concurrent `delete_acked` must never panic and
/// must never return corrupted data. The `delete_acked` path calls
/// `scan_segments` then `invalidate_scan_cache`, racing the reader's
/// own `scan_segments` + `read_segment`.
///
/// If `delete_acked` removes a segment between the reader's scan and
/// its `read_bytes`, the read returns `Err(NotFound)` — the documented
/// concurrent-delete race. The test treats `Err` as a valid outcome
/// (not data corruption) and only asserts integrity of successful reads.
#[test]
fn read_from_concurrent_delete_acked_scan_cache_no_corruption() {
    loom::model(|| {
        let buf = open_with_mock(loom_config());

        // Pre-populate: two segments [0..=1] and [2..=3].
        for i in 0..2u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();
        for i in 2..4u64 {
            buf.append(Item { id: i }).unwrap();
        }
        buf.flush().unwrap();

        // Thread A: read_from — may see 0, 2, or 4 items depending on
        // whether delete_acked has run. Must never see wrong data.
        let b1 = buf.clone();
        let h1 = thread::spawn(move || b1.read_from(0, 100));

        // Thread B: delete_acked(1) — removes segment [0..=1].
        let b2 = buf.clone();
        let h2 = thread::spawn(move || b2.delete_acked(1).unwrap());

        let batch_result = h1.join().unwrap();
        h2.join().unwrap();

        // If read_from succeeded, every item must be valid and ordered.
        // An Err(NotFound) is the documented concurrent-delete race —
        // not corruption, so we accept it.
        if let Ok(batch) = batch_result {
            let mut prev: Option<u64> = None;
            for item in &batch {
                assert!(
                    item.id < 4,
                    "phantom item: id={} (valid range 0..=3)",
                    item.id
                );
                if let Some(p) = prev {
                    assert!(
                        item.id > p,
                        "out-of-order or duplicate: {} after {}",
                        item.id,
                        p
                    );
                }
                prev = Some(item.id);
            }
        }

        // After settling: invalidate the cache via a flush so the next
        // read_from does a fresh scan (the scan-cache publication race may
        // have left a stale entry referencing the deleted segment, which
        // would cause read_from to return Err(NotFound) — the documented
        // concurrent-delete race). After invalidation, all surviving items
        // must be visible and deleted items must not reappear.
        buf.append(Item { id: 99 }).unwrap();
        buf.flush().unwrap();
        let all = buf.read_from(0, 100).unwrap();
        let ids: Vec<u64> = all.iter().map(|i| i.id).collect();
        assert!(
            ids.contains(&2) && ids.contains(&3),
            "surviving items 2,3 must be visible after settling, got {ids:?}"
        );
        assert!(
            !ids.contains(&0) && !ids.contains(&1),
            "deleted items 0,1 must not reappear after settling, got {ids:?}"
        );
    });
}
