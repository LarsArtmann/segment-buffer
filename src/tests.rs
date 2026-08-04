// Test modules override the library's strict lints. In test code, panicking
// on unexpected conditions (unwrap/expect) is the correct behavior — a test
// failure should be loud and immediate, not a silent error return. Likewise,
// `as` conversions and arithmetic on counters/indices are safe in tests, and
// the full pedantic/nursery style groups are not worth the churn.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic_in_result_fn,
    clippy::panic,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]
use super::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Barrier;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct TestItem {
    id: u64,
    payload: String,
}

fn test_item(n: u64) -> TestItem {
    TestItem {
        id: n,
        payload: format!("payload-{n}"),
    }
}

type TestBuffer = SegmentBuffer<TestItem>;

/// Shared test config: small batch, auto-flush effectively disabled. Only
/// `max_size_bytes` varies between tests, so it is the single parameter.
fn test_config(max_size_bytes: u64) -> SegmentConfig {
    SegmentConfig {
        flush_policy: FlushPolicy::Batch(4),
        max_size_bytes,
        compression_level: 3,
        durability: DurabilityPolicy::Segment,
        cipher: None,
    }
}

fn test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(dir, test_config(1024 * 1024)).expect("Failed to create buffer")
}

/// Buffer with `max_size_bytes=1000` so pressure percentages are exact.
fn pressure_test_buffer(dir: &Path) -> TestBuffer {
    SegmentBuffer::open(dir, test_config(1000)).expect("Failed to create pressure-test buffer")
}

fn set_disk_bytes<T>(buf: &SegmentBuffer<T>, bytes: u64) {
    buf.approx_disk_bytes
        .store(bytes, std::sync::atomic::Ordering::Relaxed);
}

// =========================================================================
// Filename parsing
// =========================================================================

#[test]
fn parse_filename_roundtrip() {
    use super::segment::parse_filename;

    let range = parse_filename("seg_000000000000_000000000255.zst").unwrap();
    assert_eq!(range.start, 0);
    assert_eq!(range.end, 255);

    let range = parse_filename("seg_000000001000_000000001099.zst").unwrap();
    assert_eq!(range.start, 1000);
    assert_eq!(range.end, 1099);

    assert!(parse_filename("not_a_segment").is_none());
    assert!(parse_filename("seg_000000000000.zst").is_none());
}

// =========================================================================
// Basic append / flush / read
// =========================================================================

#[test]
fn append_and_flush() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }
    assert_eq!(buf.pending_count(), 3);

    buf.flush().unwrap();
    assert_eq!(buf.pending_count(), 3);

    let segments = buf.scan_segments().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].start, 0);
    assert_eq!(segments[0].end, 2);
}

#[test]
fn flush_preserves_unflushed_capacity_for_next_batch() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: None,
        },
    )
    .expect("Failed to create buffer");

    // Append a batch of 100 items, then flush.
    for i in 0..100 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();

    // After flush, `unflushed` should have been pre-allocated with the
    // previous batch's capacity, so subsequent appends don't trigger
    // log2(N) reallocs.
    let inner = buf.inner.lock();
    assert!(
        inner.unflushed.capacity() >= 100,
        "expected recycled capacity >= 100 after flush, got {}; \
         the per-flush realloc regression returned",
        inner.unflushed.capacity()
    );
}

#[test]
fn auto_flush_at_batch_threshold() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let segments = buf.scan_segments().unwrap();
    assert_eq!(segments.len(), 1, "Should auto-flush at batch threshold");
}

#[test]
fn read_from_returns_flushed_events() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 5);
}

#[test]
fn read_from_partial_segment() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(2, 100).unwrap();
    assert_eq!(events.len(), 2, "Should skip first 2 events in segment");
}

#[test]
fn read_from_with_limit() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..6 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 3).unwrap();
    assert_eq!(events.len(), 3);
}

#[test]
fn delete_acked_removes_segments() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..8 {
        buf.append(test_item(i)).unwrap();
    }

    let deleted = buf.delete_acked(3).unwrap();
    assert_eq!(deleted, 1, "Should delete segment [0-3]");

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 4, "Should only have events 4-7");
}

#[test]
fn delete_acked_all() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    let deleted = buf.delete_acked(3).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(buf.pending_count(), 0);
}

#[test]
fn delete_acked_with_unflushed_pending_keeps_backlog_honest() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);

    // Two items stay in memory: max_batch_events is 4, no auto-flush fires.
    buf.append(test_item(0)).unwrap();
    buf.append(test_item(1)).unwrap();
    assert_eq!(buf.pending_count(), 2);

    // Consumer reads them from memory, then acks past them. There is no
    // segment file to remove, so deleted == 0.
    let deleted = buf.delete_acked(100).unwrap();
    assert_eq!(deleted, 0, "Nothing was flushed, so no segment is removed");

    // The unflushed items remain in the backlog and are still readable.
    assert_eq!(
        buf.pending_count(),
        2,
        "Unflushed items must stay counted until flushed + acknowledged"
    );
    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn latest_sequence() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    assert_eq!(buf.latest_sequence(), 0);

    buf.append(test_item(0)).unwrap();
    assert_eq!(buf.latest_sequence(), 0);

    buf.append(test_item(1)).unwrap();
    assert_eq!(buf.latest_sequence(), 1);
}

// =========================================================================
// Crash recovery
// =========================================================================

#[test]
fn crash_recovery_from_segments() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = test_buffer(dir);
        for i in 0..6 {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }

    let buf2 = test_buffer(dir);
    assert_eq!(buf2.pending_count(), 6);
    assert_eq!(buf2.latest_sequence(), 5);

    let events = buf2.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 6);
}

#[test]
fn crash_recovery_loses_unflushed_events() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = test_buffer(dir);
        for i in 0..6 {
            buf.append(test_item(i)).unwrap();
        }
    }

    let buf2 = test_buffer(dir);
    assert_eq!(
        buf2.pending_count(),
        4,
        "Should only recover flushed events (pending batch lost on crash)"
    );
    assert_eq!(buf2.latest_sequence(), 3);
}

#[test]
fn crash_recovery_cleans_tmp_files() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    fs::write(
        dir.join("seg_000000000000_000000000003.zst.tmp"),
        b"incomplete",
    )
    .unwrap();

    let buf = test_buffer(dir);
    assert_eq!(buf.pending_count(), 0);
    assert!(!dir.join("seg_000000000000_000000000003.zst.tmp").exists());
}

// =========================================================================
// Roundtrip integrity
// =========================================================================

#[test]
fn read_includes_pending_events() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    for i in 4..7 {
        buf.append(test_item(i)).unwrap();
    }

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 7, "Should include 4 flushed + 3 pending");
}

#[test]
fn roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    let item = test_item(42);
    buf.append(item.clone()).unwrap();
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], item);
}

// =========================================================================
// Pressure / overload (store_pressure stays in the crate; should_accept is removed)
// =========================================================================

#[test]
fn store_pressure_returns_0_when_no_limit() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(tmp.path(), test_config(0)).expect("create buffer");
    assert_eq!(buf.store_pressure(), 0.0);
    assert!(!buf.is_overloaded());
}

#[test]
fn store_pressure_bounded_at_1_0_when_disk_exceeds_limit() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(tmp.path(), test_config(1)).expect("create buffer");
    set_disk_bytes(&buf, 999_999_999);
    let pressure = buf.store_pressure();
    assert!(
        (pressure - 1.0).abs() < f32::EPSILON,
        "Pressure should be clamped to 1.0, got {pressure}"
    );
    assert!(buf.is_overloaded());
}

#[test]
fn is_overloaded_true_above_90_percent() {
    let tmp = TempDir::new().unwrap();
    let buf = pressure_test_buffer(tmp.path());
    set_disk_bytes(&buf, 901); // 90.1%
    assert!(buf.is_overloaded());
}

#[test]
fn is_overloaded_false_at_or_below_90_percent() {
    let tmp = TempDir::new().unwrap();
    let buf = pressure_test_buffer(tmp.path());
    set_disk_bytes(&buf, 900); // exactly 90%
    assert!(
        !buf.is_overloaded(),
        "is_overloaded is pressure > 0.9, not >="
    );
}

// =========================================================================
// Concurrency stress test — 4 writers + 1 reader, 10K events
// =========================================================================

#[test]
fn concurrency_4_writers_1_reader_10k_events() {
    let tmp = TempDir::new().unwrap();
    // FlushPolicy::Manual keeps all items in-memory during the concurrent phase.
    // The purpose is to stress-test append/read correctness under contention,
    // not disk I/O. With Batch(4) this test would create 2_500 segment files.
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 2_500;
    const TOTAL: usize = WRITERS * PER_WRITER; // 10_000

    let latest_seen = Arc::new(Mutex::new(0u64));

    thread::scope(|s| {
        // Reader thread: polls read_from until all events seen
        let buf_r = Arc::clone(&buf);
        let latest_r = Arc::clone(&latest_seen);
        s.spawn(move || loop {
            let start = *latest_r.lock();
            if start >= TOTAL as u64 {
                break;
            }
            if let Ok(events) = buf_r.read_from(start, 500) {
                if !events.is_empty() {
                    *latest_r.lock() = start + events.len() as u64;
                }
            }
            thread::sleep(Duration::from_micros(50));
        });

        // 4 writer threads, each appending 2_500 events
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            s.spawn(move || {
                for i in 0..PER_WRITER {
                    let _ = buf_w.append(test_item((writer_id * PER_WRITER + i) as u64));
                }
            });
        }
    });

    // All threads joined. Flush any remaining in-memory events.
    buf.flush().unwrap();

    // Verify: all 10K events assigned, all recoverable
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);
    assert_eq!(buf.pending_count(), TOTAL as u64);

    let all_events = buf.read_from(0, TOTAL * 2).unwrap();
    assert_eq!(
        all_events.len(),
        TOTAL,
        "All {TOTAL} events should be recoverable"
    );
}

// =========================================================================
// Concurrency stress test — BatchOrIntervalMin under contention
// =========================================================================

#[test]
fn concurrency_batch_or_interval_min_4_writers_10k_events() {
    // Proves that BatchOrIntervalMin is safe under concurrent append:
    // the batch_size auto-flush trigger fires during contention without
    // corrupting sequence numbers or losing items. The interval and
    // max_interval are set very high so only the batch_size trigger fires
    // (no timing flakiness).
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::BatchOrIntervalMin {
                    batch_size: 1000,
                    min_batch: 100,
                    interval: Duration::from_secs(3600),
                    max_interval: Duration::from_secs(7200),
                },
                max_size_bytes: 100 * 1024 * 1024,
                compression_level: 1,
                durability: DurabilityPolicy::Throughput,
                cipher: None,
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 2_500;
    const TOTAL: usize = WRITERS * PER_WRITER;

    thread::scope(|s| {
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            s.spawn(move || {
                for i in 0..PER_WRITER {
                    let _ = buf_w.append(test_item((writer_id * PER_WRITER + i) as u64));
                }
            });
        }
    });

    buf.flush().unwrap();

    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);
    assert_eq!(buf.pending_count(), TOTAL as u64);

    let all_events = buf.read_from(0, TOTAL * 2).unwrap();
    assert_eq!(all_events.len(), TOTAL);
}

// =========================================================================
// Concurrent read_from + delete_acked boundary (MPMC safety)
// =========================================================================

/// Proves the MPMC read/delete boundary documented in `DOMAIN_LANGUAGE.md`'s
/// "Consistency Model → Concurrent operation" subsection.
///
/// Under concurrent `read_from` + `delete_acked`, `read_from` may return a
/// spurious `SegmentError::Io` (`NotFound`) when the deleter removes a segment
/// between the reader's scan and its file read. This is not a bug: the deleted
/// segment was already acknowledged. The invariant this test proves is that
/// `read_from` **never returns wrong data** — every item the reader
/// successfully deserializes has the correct sequence-to-value mapping.
#[test]
fn concurrent_read_and_delete_never_corrupts() {
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                max_size_bytes: 100 * 1024 * 1024,
                compression_level: 1, // minimize CPU to widen the race window
                durability: DurabilityPolicy::Throughput,
                cipher: None,
            },
        )
        .unwrap(),
    );

    // Pre-populate: 50 segments × 100 items = 5 000 items on disk.
    const PER_SEG: u64 = 100;
    const SEGMENTS: u64 = 50;
    const TOTAL: u64 = PER_SEG * SEGMENTS;
    for start in (0..TOTAL).step_by(PER_SEG as usize) {
        for i in 0..PER_SEG {
            buf.append(test_item(start + i)).unwrap();
        }
        buf.flush().unwrap();
    }

    let corruption = Arc::new(std::sync::atomic::AtomicBool::new(false));

    thread::scope(|s| {
        // Reader: scans forward, verifying every item id. Retries on empty
        // (concurrent-flush window) and skips on Io error (segment deleted
        // under us — the documented boundary).
        let buf_r = Arc::clone(&buf);
        let corrupt_r = Arc::clone(&corruption);
        s.spawn(move || {
            let mut prev_id: Option<u64> = None;
            let mut pos = 0u64;
            let mut empty_retries = 0u32;
            while pos < TOTAL {
                match buf_r.read_from(pos, 500) {
                    Ok(batch) if !batch.is_empty() => {
                        empty_retries = 0;
                        for item in &batch {
                            // Every item must be in range and strictly
                            // increasing. Gaps from deleted segments are
                            // fine; reordering or duplicates are corruption.
                            if item.id >= TOTAL || prev_id.is_some_and(|p| item.id <= p) {
                                corrupt_r.store(true, std::sync::atomic::Ordering::SeqCst);
                                return;
                            }
                            prev_id = Some(item.id);
                        }
                        pos = prev_id.unwrap() + 1;
                    }
                    Ok(_) => {
                        // Empty: items moved (flush race) or deleted ahead.
                        // Retry briefly, then advance past the gap.
                        empty_retries += 1;
                        if empty_retries > 5 {
                            pos = ((pos / PER_SEG) + 1) * PER_SEG;
                            empty_retries = 0;
                        } else {
                            thread::sleep(Duration::from_micros(100));
                        }
                    }
                    Err(_) => {
                        // Io error: segment deleted between scan and read.
                        // This is the documented MPMC boundary. Skip forward.
                        pos = ((pos / PER_SEG) + 1) * PER_SEG;
                    }
                }
            }
        });

        // Deleter: removes segments from the front, racing with the reader.
        let buf_d = Arc::clone(&buf);
        s.spawn(move || {
            for acked in (PER_SEG..TOTAL).step_by(PER_SEG as usize) {
                let _ = buf_d.delete_acked(acked);
                thread::sleep(Duration::from_micros(10));
            }
        });
    });

    assert!(
        !corruption.load(std::sync::atomic::Ordering::SeqCst),
        "read_from returned wrong data under concurrent delete_acked"
    );
}

/// Proves that `read_from` under concurrent `flush` never returns corrupt
/// data. The flush-race window (Phase 1 directory scan → Phase 2 mutex gap)
/// can cause transient gaps: items that have left `unflushed` but whose
/// segment file was not yet visible to the scan. The test asserts that reads
/// never return wrong, out-of-order, or duplicate items, and that all items
/// become visible once the flusher settles.
#[test]
fn concurrent_read_and_flush_never_corrupts() {
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                max_size_bytes: 100 * 1024 * 1024,
                compression_level: 1, // minimize CPU to widen the race window
                durability: DurabilityPolicy::Throughput,
                cipher: None,
            },
        )
        .unwrap(),
    );

    // Half the items pre-flushed to disk, half left in `unflushed` (Manual
    // policy keeps them in memory until flush is called).
    const ON_DISK: u64 = 500;
    const IN_MEMORY: u64 = 500;
    const TOTAL: u64 = ON_DISK + IN_MEMORY;

    for i in 0..ON_DISK {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    for i in ON_DISK..TOTAL {
        buf.append(test_item(i)).unwrap();
    }

    let corruption = Arc::new(std::sync::atomic::AtomicBool::new(false));

    thread::scope(|s| {
        // Reader: scans forward, verifying every item id. Tolerates transient
        // gaps (flush race) by retrying, but fails on any wrong, backwards, or
        // duplicate item.
        let buf_r = Arc::clone(&buf);
        let corrupt_r = Arc::clone(&corruption);
        s.spawn(move || {
            let mut pos = 0u64;
            let mut prev_id: Option<u64> = None;
            let mut empty_retries = 0u32;
            while pos < TOTAL {
                match buf_r.read_from(pos, 200) {
                    Ok(batch) if !batch.is_empty() => {
                        empty_retries = 0;
                        for item in &batch {
                            if item.id >= TOTAL || prev_id.is_some_and(|p| item.id <= p) {
                                corrupt_r.store(true, std::sync::atomic::Ordering::SeqCst);
                                return;
                            }
                            prev_id = Some(item.id);
                        }
                        pos = prev_id.unwrap() + 1;
                    }
                    Ok(_) => {
                        // Transient gap: items left `unflushed` but their
                        // segment was not yet scanned. Retry — the gap closes
                        // once the segment lands on disk.
                        empty_retries += 1;
                        if empty_retries > 200 {
                            break;
                        }
                        thread::sleep(Duration::from_micros(20));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_micros(20));
                    }
                }
            }
        });

        // Flusher: drains `unflushed` to disk, racing with the reader's
        // scan → lock gap.
        let buf_f = Arc::clone(&buf);
        s.spawn(move || {
            for _ in 0..20 {
                let _ = buf_f.flush();
                thread::sleep(Duration::from_micros(50));
            }
        });
    });

    assert!(
        !corruption.load(std::sync::atomic::Ordering::SeqCst),
        "read_from returned wrong data under concurrent flush"
    );

    // After the flusher settles, all items must be visible — the transient
    // gap closes once the segment is durably on disk.
    buf.flush().unwrap();
    let all = buf.read_from(0, TOTAL as usize + 10).unwrap();
    assert_eq!(
        all.len() as u64,
        TOTAL,
        "all items must be readable after flush completes"
    );
    for (i, item) in all.iter().enumerate() {
        assert_eq!(item.id, i as u64, "item at position {i} has wrong id");
    }
}

// =========================================================================
// Deterministic scan-cache TOCTOU regression test
// =========================================================================
//
// The scan-cache mtime-ordering fix (commit dc7ea7a) ensures that the
// directory mtime is captured BEFORE the readdir, not after. Without this
// ordering, a segment rename landing mid-scan pairs a post-rename mtime
// with a pre-rename (stale) segment list in the cache: the staleness guard
// then sees "no change" and serves stale data indefinitely.
//
// The test below forces the EXACT interleaving deterministically using
// `std::sync::Barrier` — no `thread::sleep`, no retry loop, no reliance on
// scheduler timing. A `HookedStore` wrapping `RealStore` injects two
// barrier wait-points into `scan()`:
//
//   1. After the readdir completes (stale snapshot captured)
//   2. Before returning that snapshot to the caller
//
// Between those two points, the mutator thread does `append + flush`,
// creating a new segment file whose rename changes the directory mtime.
// The scan returns the stale list; `scan_segments` publishes it with a
// pre-rename mtime. The NEXT `read_from` must detect the mtime change via
// `dir_mtime_changed()` and force a re-scan, recovering the missing segment.
//
// If the fix is reverted (mtime captured after the scan), the cached mtime
// would match the post-rename directory mtime, the guard would see "no
// change," and the second read would serve stale data — the assertion on
// `second.len() == 11` fails.

/// A `RealStore` wrapper whose `scan()` method uses barriers to force a
/// deterministic interleaving with a concurrent mutation. Used exclusively
/// by the scan-cache TOCTOU regression test.
///
/// The `barrier_armed` flag is set by the test harness AFTER `open_internal`
/// (whose `recover()` call does the first scan) and BEFORE the racing
/// `read_from`. This ensures exactly one scan — the second ever — passes
/// through the barrier dance. All other scans are pass-throughs to the
/// inner `RealStore`.
struct HookedStore {
    inner: store::RealStore,
    /// One-shot flag: the next `scan()` call blocks on the barriers.
    /// Set by the test after open, consumed (swapped to false) by the
    /// first scan that observes it.
    barrier_armed: std::sync::atomic::AtomicBool,
    /// Fired after `scan()` has completed its readdir (stale snapshot
    /// captured). Signals the mutator: "the directory has been read;
    /// you may now flush."
    scan_done_barrier: Arc<Barrier>,
    /// Fired after the mutator has completed its flush. Signals the
    /// scanner: "the rename has landed; you may return the stale result."
    mutation_done_barrier: Arc<Barrier>,
}

impl HookedStore {
    fn new(
        dir: PathBuf,
        scan_done_barrier: Arc<Barrier>,
        mutation_done_barrier: Arc<Barrier>,
    ) -> Self {
        Self {
            inner: store::RealStore::new(dir),
            barrier_armed: std::sync::atomic::AtomicBool::new(false),
            scan_done_barrier,
            mutation_done_barrier,
        }
    }
}

impl store::SegmentStore for HookedStore {
    fn create_dir_all(&self) -> super::Result<()> {
        self.inner.create_dir_all()
    }

    fn scan(&self) -> super::Result<Vec<super::segment::SegmentRange>> {
        // Capture the readdir result IMMEDIATELY — before any barrier.
        // This is the stale snapshot: the mutator has not yet flushed.
        let result = self.inner.scan()?;
        // Only the scan that finds the flag armed does the barrier dance.
        // swap returns the previous value; if it was true, we proceed.
        if self
            .barrier_armed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            // Signal the mutator: readdir is done, flush now.
            self.scan_done_barrier.wait();
            // Wait for the mutator: flush is complete, the rename has
            // landed, the directory mtime has changed.
            self.mutation_done_barrier.wait();
        }
        // Return the stale snapshot captured before the flush.
        Ok(result)
    }

    fn clean_tmp(&self) -> super::Result<usize> {
        self.inner.clean_tmp()
    }

    fn segment_size(&self, range: super::segment::SegmentRange) -> u64 {
        self.inner.segment_size(range)
    }

    fn remove_segment(&self, range: super::segment::SegmentRange) -> super::Result<bool> {
        self.inner.remove_segment(range)
    }

    fn write_atomic(
        &self,
        range: super::segment::SegmentRange,
        payload: &[u8],
        policy: DurabilityPolicy,
    ) -> super::Result<u64> {
        self.inner.write_atomic(range, payload, policy)
    }

    fn read_bytes(&self, range: super::segment::SegmentRange) -> super::Result<Vec<u8>> {
        self.inner.read_bytes(range)
    }
}

/// Deterministic regression test for the scan-cache TOCTOU fix.
///
/// Forces the exact `scan → rename → scan-returns-stale → cache-populate`
/// interleaving via barriers, then verifies the mtime guard detects the
/// change and forces a re-scan on the next call.
///
/// **Without the fix** (mtime captured after scan): the second read serves
/// stale cached data (10 items instead of 11) and the assertion fails.
///
/// **With the fix** (mtime captured before scan): the second read detects
/// the mtime change, re-scans, and returns all 11 items.
#[test]
fn scan_cache_toctou_mtime_guard_forces_rescan_after_mid_scan_rename() {
    use std::sync::Barrier;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let scan_done = Arc::new(Barrier::new(2));
    let mutation_done = Arc::new(Barrier::new(2));

    let hooked = Arc::new(HookedStore::new(
        dir.clone(),
        scan_done.clone(),
        mutation_done.clone(),
    ));
    let store_handle = hooked.clone();
    let store: Arc<dyn store::SegmentStore + Send + Sync> = hooked;

    let config = SegmentConfig {
        flush_policy: FlushPolicy::Manual,
        max_size_bytes: 100 * 1024 * 1024,
        compression_level: 1, // minimise CPU to keep the race window tight
        durability: DurabilityPolicy::Throughput,
        cipher: None,
    };

    // open_internal calls recover() which does the first scan (pass-through;
    // barrier_armed is false at this point). Lock file is None — we are the
    // sole owner, but we skip the flock to match the HookedStore pattern.
    let (buf, _report) = SegmentBuffer::<TestItem>::open_internal(dir, config, store, None)
        .expect("open_internal must succeed");

    // The mtime guard is the mechanism under test. If the filesystem does
    // not support mtime (rare: coarse-granularity FUSE / network FS), the
    // guard is disabled and this test cannot exercise it.
    assert!(
        buf.mtime_supported,
        "this test requires filesystem mtime support; \
         the open-time capability probe reported mtime as unsupported"
    );

    // Pre-populate: 10 items flushed as one segment [0..=9].
    for i in 0..10u64 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // flush() invalidates the cache → the next read_from triggers a scan.

    // Arm the barrier: the NEXT scan (the one inside the racing read_from)
    // will block on the barriers.
    store_handle
        .barrier_armed
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let buf = Arc::new(buf);

    let (first_count, second_count) = thread::scope(|s| {
        // Thread A: read_from triggers scan_segments → HookedStore.scan().
        // The HookedStore captures the stale readdir, barriers with Thread B,
        // and returns the stale list. The first read sees only [0..=9].
        // The second read must see [0..=10] via the mtime guard.
        let buf_reader = Arc::clone(&buf);
        let reader = s.spawn(move || {
            let first = buf_reader.read_from(0, 100).unwrap();
            let second = buf_reader.read_from(0, 100).unwrap();
            (first.len(), second.len())
        });

        // Thread B: append + flush during Thread A's scan.
        let buf_mutator = Arc::clone(&buf);
        s.spawn(move || {
            // Wait for Thread A's scan to complete its readdir.
            scan_done.wait();
            // Flush a new segment [10..=10]. The rename changes the dir mtime.
            buf_mutator.append(test_item(10)).unwrap();
            buf_mutator.flush().unwrap();
            // Signal Thread A: the mutation is complete.
            mutation_done.wait();
        });

        reader.join().unwrap()
    });

    // First read returned only the pre-flush segment: 10 items.
    // The scan captured the stale readdir before the flush.
    assert_eq!(
        first_count, 10,
        "first read should see only the pre-flush segments (stale scan)"
    );

    // Second read MUST see all 11 items: the mtime guard detected the
    // directory change (pre-scan mtime ≠ post-flush mtime) and forced a
    // re-scan. If the fix is reverted, this assertion fails with 10 items.
    assert_eq!(
        second_count, 11,
        "second read must recover the flushed segment via the mtime guard \
         (pre-scan mtime was stale, forcing a re-scan)"
    );
}

// =========================================================================
// Time-based auto-flush
// =========================================================================

// ---------------------------------------------------------------------
// FlushPolicy time-based decision tests (pure, no wall-clock dependency)
// ---------------------------------------------------------------------
//
// These tests call FlushPolicy::should_flush directly with synthetic time
// values instead of going through open() → append() → thread::sleep →
// check files. This eliminates CI flakiness from scheduler jitter and makes
// the exact decision boundary precisely testable.
//
// The integration path (should_flush → flush → segment file on disk) is
// already covered by auto_flush_at_batch_threshold and
// batch_or_interval_min_flushes_at_batch_size below, which trigger via
// batch_size (instantaneous, no timing dependency).

#[test]
fn batch_or_interval_flushes_after_interval() {
    let policy = FlushPolicy::BatchOrInterval {
        batch_size: 256,
        interval: Duration::from_secs(5),
    };
    // Below batch_size, before interval: no flush
    assert!(!policy.should_flush(10, Duration::from_secs(4)));
    assert!(!policy.should_flush(255, Duration::from_secs(4)));
    // Below batch_size, at interval: flush
    assert!(policy.should_flush(10, Duration::from_secs(5)));
    // Below batch_size, well past interval: flush
    assert!(policy.should_flush(1, Duration::from_secs(10)));
}

// =========================================================================
// FlushPolicy::BatchOrIntervalMin tests
// =========================================================================

#[test]
fn batch_or_interval_min_suppresses_small_flush() {
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 256,
        min_batch: 10,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(60),
    };
    // Below min_batch, even past interval but before max_interval: NO flush
    assert!(!policy.should_flush(1, Duration::from_secs(10)));
    assert!(!policy.should_flush(9, Duration::from_secs(10)));
    assert!(!policy.should_flush(9, Duration::from_secs(59)));
    // Zero pending: never flush (unless past max_interval)
    assert!(!policy.should_flush(0, Duration::from_secs(59)));
}

#[test]
fn batch_or_interval_min_flushes_at_min_batch() {
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 256,
        min_batch: 10,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(60),
    };
    // At min_batch AND past interval: flush
    assert!(policy.should_flush(10, Duration::from_secs(5)));
    assert!(policy.should_flush(10, Duration::from_secs(6)));
    // Well above min_batch, past interval: flush
    assert!(policy.should_flush(50, Duration::from_secs(10)));
}

#[test]
fn batch_or_interval_min_flushes_at_max_interval() {
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 256,
        min_batch: 100,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(60),
    };
    // Below min_batch AND below batch_size, but past max_interval: flush (safety valve)
    assert!(policy.should_flush(1, Duration::from_secs(60)));
    assert!(policy.should_flush(0, Duration::from_secs(120)));
    // Before max_interval and below min_batch: no flush
    assert!(!policy.should_flush(1, Duration::from_secs(59)));
}

#[test]
fn batch_or_interval_min_flushes_at_batch_size() {
    let tmp = TempDir::new().unwrap();
    let buf: TestBuffer = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::BatchOrIntervalMin {
                batch_size: 4,
                min_batch: 100,
                interval: std::time::Duration::from_secs(30),
                max_interval: std::time::Duration::from_secs(60),
            },
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: None,
        },
    )
    .expect("create buffer");

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }

    assert_eq!(
        buf.scan_segments().unwrap().len(),
        1,
        "Should flush immediately at batch_size regardless of min_batch/interval"
    );
}

#[test]
fn batch_or_interval_min_min_batch_zero_always_flushes_at_interval() {
    // When min_batch == 0, `pending_len >= 0` is always true, so the
    // interval trigger fires as soon as `interval` elapses — equivalent to
    // BatchOrInterval for the interval arm.
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 256,
        min_batch: 0,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(60),
    };
    // Any pending count, at interval: flush (because 0 >= 0 is true)
    assert!(policy.should_flush(0, Duration::from_secs(5)));
    assert!(policy.should_flush(1, Duration::from_secs(5)));
    // Before interval: no flush (unless batch_size or max_interval hit)
    assert!(!policy.should_flush(0, Duration::from_secs(4)));
}

#[test]
fn batch_or_interval_min_max_equals_interval_ignores_min_batch() {
    // When max_interval == interval, the safety valve fires at the same time
    // as the interval trigger, so min_batch becomes irrelevant — every
    // interval expiry is a guaranteed flush.
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 256,
        min_batch: 100,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(5),
    };
    // Below min_batch, but at interval (= max_interval): flush via safety valve
    assert!(policy.should_flush(1, Duration::from_secs(5)));
    assert!(policy.should_flush(0, Duration::from_secs(5)));
    // Before interval: no flush
    assert!(!policy.should_flush(50, Duration::from_secs(4)));
}

#[test]
fn batch_or_interval_min_min_batch_equals_batch_size() {
    // When min_batch == batch_size, the interval trigger requires a full
    // batch before firing — so the interval arm reduces to the batch arm,
    // and only max_interval can flush below batch_size.
    let policy = FlushPolicy::BatchOrIntervalMin {
        batch_size: 100,
        min_batch: 100,
        interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(60),
    };
    // Below batch_size, past interval but before max_interval: no flush
    // (min_batch == batch_size means the interval check also requires 100)
    assert!(!policy.should_flush(99, Duration::from_secs(10)));
    assert!(!policy.should_flush(99, Duration::from_secs(59)));
    // At batch_size: flush regardless of time
    assert!(policy.should_flush(100, Duration::from_secs(0)));
    // Past max_interval: flush regardless of pending count
    assert!(policy.should_flush(0, Duration::from_secs(60)));
}

// =========================================================================
// FlushPolicy Display tests
// =========================================================================

#[test]
fn flush_policy_display_formats_each_variant() {
    assert_eq!(FlushPolicy::Batch(256).to_string(), "batch(256)");
    assert_eq!(
        FlushPolicy::Interval(Duration::from_secs(5)).to_string(),
        "interval(5s)"
    );
    assert_eq!(
        FlushPolicy::BatchOrInterval {
            batch_size: 256,
            interval: Duration::from_secs(5),
        }
        .to_string(),
        "batch_or_interval(batch=256, interval=5s)"
    );
    assert_eq!(
        FlushPolicy::BatchOrIntervalMin {
            batch_size: 256,
            min_batch: 10,
            interval: Duration::from_secs(5),
            max_interval: Duration::from_secs(60),
        }
        .to_string(),
        "batch_or_interval_min(batch=256, min=10, interval=5s, max=60s)"
    );
    assert_eq!(FlushPolicy::Manual.to_string(), "manual");
}

// =========================================================================
// Error-path tests (no encryption)
// =========================================================================

#[test]
fn corrupted_zstd_segment_returns_error_not_panic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let garbage_path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&garbage_path, b"this is not valid zstd data at all").unwrap();

    let buf = test_buffer(dir);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Corrupted zstd segment should return an error, not panic"
    );
}

#[test]
fn legacy_envelopeless_file_still_reads() {
    use super::segment;
    // Hand-build a v1-format file (no SBF1 envelope), exactly as monitor365
    // would have written it: raw zstd(CBOR), no envelope prefix.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let items = vec![test_item(7), test_item(8)];
    let mut cbor = Vec::new();
    ciborium::into_writer(&items, &mut cbor).unwrap();
    let raw_v1 = zstd::encode_all(cbor.as_slice(), 3).unwrap();

    let path = dir.join(segment::filename(7, 8));
    fs::write(&path, &raw_v1).unwrap();

    // Read via the buffer (no cipher). The envelope-less bytes should be
    // detected as legacy and decoded transparently.
    let buf = test_buffer(dir);
    let events: Vec<TestItem> = buf.read_from(7, 100).unwrap();
    assert_eq!(events.len(), 2, "legacy envelope-less file must still read");
    assert_eq!(events[0], test_item(7));
}

#[test]
fn enveloped_file_roundtrips_and_carries_magic() {
    use super::segment;
    const ENVELOPE_MAGIC: &[u8; 4] = b"SBF1";

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);

    buf.append(test_item(1)).unwrap();
    buf.append(test_item(2)).unwrap();
    buf.flush().unwrap();

    // The file on disk must start with the SBF1 magic. Sequence numbers are
    // assigned by the buffer (0-based), so two appends → filename(0, 1).
    let path = dir.join(segment::filename(0, 1));
    assert!(path.exists(), "segment file should exist at {path:?}");
    let bytes = fs::read(&path).unwrap();
    assert!(
        bytes.len() >= 8,
        "enveloped file should be at least header-length"
    );
    assert_eq!(
        &bytes[..4],
        ENVELOPE_MAGIC,
        "newly-written segment must carry the SBF1 envelope magic"
    );

    // And it must round-trip cleanly.
    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn envelope_detection_requires_zero_reserved_bytes() {
    use super::segment::{unwrap_envelope, wrap_envelope};

    // Sanity: the canonical envelope (zero reserved) is detected.
    let wrapped = wrap_envelope(b"payload");
    assert!(matches!(unwrap_envelope(&wrapped), (Some(1), _)));

    // A v1-shape block whose reserved bytes are NON-zero must NOT be treated
    // as an envelope, even though the magic matches. This is the hardening:
    // a legacy encrypted file whose AEAD nonce begins with `SBF1` followed
    // by three non-zero bytes (~2⁻³² of files) would otherwise be silently
    // mis-framed as an envelope. Requiring reserved-zero drops the false
    // positive to 2⁻⁵⁶.
    let mut looks_like_envelope = vec![b'S', b'B', b'F', b'1', 1, 0xFF, 0xFF, 0xFF];
    looks_like_envelope.extend_from_slice(b"payload");
    let (version, payload) = unwrap_envelope(&looks_like_envelope);
    assert_eq!(
        version, None,
        "magic with non-zero reserved bytes must not be detected as envelope"
    );
    assert_eq!(
        payload,
        looks_like_envelope.as_slice(),
        "non-conforming bytes must pass through unmodified as legacy"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn legacy_encrypted_file_without_envelope_still_reads() {
    // The headline monitor365 byte-compatibility guarantee: a segment file
    // written by monitor365 (no SBF1 envelope, just `[nonce][ciphertext]`)
    // must read back transparently through the enveloped reader when the
    // matching cipher is configured. This was previously untested.
    use super::segment;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let items = vec![test_item(101), test_item(102), test_item(103)];

    // Encode the v1 payload exactly as monitor365 would have: CBOR → zstd →
    // AEAD-encrypt. Then write the raw payload bytes (NO envelope) under a
    // valid segment filename.
    let key = [0xABu8; 32];
    let cipher = AesGcmCipher::new(&key);
    let path = dir.join(segment::filename(101, 103));
    let mut compressor = zstd::bulk::Compressor::new(3).unwrap();
    let payload = segment::encode_payload(Some(&cipher), &mut compressor, &path, &items).unwrap();
    assert!(
        !payload.starts_with(b"SBF1"),
        "raw encrypted payload must not accidentally carry the magic"
    );
    fs::write(&path, &payload).unwrap();

    // Open the buffer with the same cipher and read the segment back.
    let buf = encrypted_buffer(dir, key);
    let events: Vec<TestItem> = buf.read_from(101, 100).unwrap();
    assert_eq!(
        events, items,
        "legacy encrypted file (no envelope) must decode transparently"
    );
}

// =========================================================================
// Encryption tests (behind `encryption` feature)
// =========================================================================

#[cfg(feature = "encryption")]
fn encrypted_buffer(dir: &Path, key: [u8; 32]) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            flush_policy: FlushPolicy::Batch(4),
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: Some(Arc::new(AesGcmCipher::new(&key))),
        },
    )
    .expect("Failed to create encrypted buffer")
}

#[cfg(feature = "encryption")]
#[test]
fn encrypted_roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer(tmp.path(), [0u8; 32]);

    let item = test_item(99);
    buf.append(item.clone()).unwrap();
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], item);

    // Verify the segment file on disk is NOT plaintext. Filter to .zst
    // because the directory also contains the `.segment-buffer.lock`
    // sidecar (held open by the flock since v0.5.0); the old
    // `read_dir().next()` form grabbed whichever entry the kernel
    // returned first, which is fs-dependent and sometimes the lock file.
    let segment_path = tmp
        .path()
        .read_dir()
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "zst"))
        .expect("segment file must exist after flush");
    let raw = fs::read(&segment_path).unwrap();
    assert!(
        raw.len() > 12,
        "Encrypted segment should be nonce + ciphertext, not plaintext"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn truncated_encrypted_segment_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&path, [0u8; 11]).unwrap();

    let buf = encrypted_buffer(dir, [0u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Truncated encrypted segment (<12 bytes) should return an error"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn encrypted_segment_nonce_only_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let path = dir.join("seg_000000000000_000000000000.zst");
    fs::write(&path, [0u8; 12]).unwrap();

    let buf = encrypted_buffer(dir, [0u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Encrypted segment with nonce but no ciphertext should return an error"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn wrong_decryption_key_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    let buf = encrypted_buffer(dir, [1u8; 32]);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Wrong decryption key should fail to read encrypted segment"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn decrypt_without_key_returns_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    // Reopen WITHOUT cipher — tries to zstd-decode ciphertext → fails
    let buf = test_buffer(dir);
    let result = buf.read_from(0, 100);
    assert!(
        result.is_err(),
        "Reading encrypted segment without a cipher should fail"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn wrong_key_cipher_error_carries_source_chain() {
    // The cipher error surfaced to the caller must keep the underlying AEAD
    // failure reachable via `std::error::Error::source`, so operators can
    // inspect the original decryption failure instead of just a flat string.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    {
        let buf = encrypted_buffer(dir, [0u8; 32]);
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
    }

    let buf = encrypted_buffer(dir, [1u8; 32]);
    let err = buf.read_from(0, 100).expect_err("wrong key must error");

    let super::SegmentError::Cipher { message, .. } = &err else {
        panic!("expected Cipher variant, got {err:?}");
    };
    assert!(
        message.contains("AES-GCM decryption failed"),
        "message should name the phase, got: {message}"
    );
    // The CipherError's source chain was lost when promoted to SegmentError::Cipher
    // (the variant stores a flat String), but the underlying AEAD failure must
    // still be reachable on the CipherError itself. We exercise that path via
    // a direct cipher call.
    use super::SegmentCipher;
    let cipher = AesGcmCipher::new(&[0u8; 32]);
    let bad_payload = [0u8; 64]; // plausible size, wrong bytes
    let cipher_err = cipher.decrypt(&bad_payload).unwrap_err();
    assert!(
        std::error::Error::source(&cipher_err).is_some(),
        "CipherError from AES-GCM must expose the AEAD failure via source()"
    );
}

// =========================================================================
// XChaCha20-Poly1305 cipher (encryption feature)
// =========================================================================

#[cfg(feature = "encryption")]
fn encrypted_buffer_xchacha(dir: &Path, key: [u8; 32]) -> TestBuffer {
    SegmentBuffer::open(
        dir,
        SegmentConfig {
            flush_policy: FlushPolicy::Batch(4),
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: Some(Arc::new(XChaCha20Poly1305Cipher::new(&key))),
        },
    )
    .expect("Failed to create XChaCha20-encrypted buffer")
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_roundtrip_preserves_event_data() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer_xchacha(tmp.path(), [0u8; 32]);

    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();

    let events = buf.read_from(0, 100).unwrap();
    assert_eq!(events.len(), 5);
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.id, i as u64);
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_cipher_roundtrip_direct() {
    // Direct trait-level roundtrip independent of the buffer: encrypt + decrypt
    // must reproduce the input for arbitrary plaintexts.
    let cipher = XChaCha20Poly1305Cipher::new(&[0xau8; 32]);
    for plaintext in [b"".as_slice(), b"x", b"hello world", &[42u8; 4096]] {
        let ct = cipher.encrypt(plaintext).expect("encrypt");
        // The ciphertext must include the 24-byte nonce prefix.
        assert!(ct.len() >= 24, "ciphertext must include nonce prefix");
        let pt = cipher.decrypt(&ct).expect("decrypt");
        assert_eq!(pt, plaintext, "roundtrip must reproduce plaintext");
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_tamper_detection() {
    // Flip one byte of the ciphertext → AEAD tag must fail verification.
    let cipher = XChaCha20Poly1305Cipher::new(&[0xbu8; 32]);
    let mut ct = cipher.encrypt(b"secret payload").expect("encrypt");
    // Flip the last byte (inside the Poly1305 tag region).
    let last = ct.len() - 1;
    ct[last] ^= 0x01;
    let err = cipher
        .decrypt(&ct)
        .expect_err("tampered ciphertext must fail AEAD");
    assert!(
        err.to_string().contains("XChaCha20"),
        "error should name XChaCha20: got {err}"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_short_payload_rejected() {
    // Payload shorter than the 24-byte nonce prefix must be rejected before
    // the AEAD is invoked, with a clear CipherError (not an opaque AEAD error).
    let cipher = XChaCha20Poly1305Cipher::new(&[0xcu8; 32]);
    for short_len in 0..24 {
        let payload = vec![0u8; short_len];
        let err = cipher
            .decrypt(&payload)
            .expect_err("sub-nonce payload must error");
        assert!(
            err.to_string().contains("nonce"),
            "error should mention nonce: got {err}"
        );
    }
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_buffer_segment_roundtrip_with_delete_acked() {
    let tmp = TempDir::new().unwrap();
    let buf = encrypted_buffer_xchacha(tmp.path(), [0xdu8; 32]);

    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    // Batch(4) triggers auto-flush on the 4th append.
    assert_eq!(buf.pending_count(), 4);

    // Acknowledge the first 3 items; one segment [0..=3] is too tall to
    // ack with seq=2, so the segment survives.
    let removed = buf.delete_acked(2).unwrap();
    assert_eq!(removed, 0);
    // Ack all 4: segment [0..=3] is fully covered.
    let removed = buf.delete_acked(3).unwrap();
    assert_eq!(removed, 1);
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_recommended_cipher_installs_xchacha() {
    // The recommended_cipher() builder helper must install an XChaCha20
    // cipher (the documented direction for new buffers).
    let cfg = SegmentConfig::builder()
        .recommended_cipher([0xeu8; 32])
        .build();
    assert!(
        cfg.cipher.is_some(),
        "recommended_cipher must install a cipher"
    );
    // Smoke: the cipher works for a roundtrip via the buffer.
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(tmp.path(), cfg).unwrap();
    buf.append(test_item(7)).unwrap();
    buf.flush().unwrap();
    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items, vec![test_item(7)]);
}

// =========================================================================
// Single-process flock (M2)
// =========================================================================

/// Second `open()` on the same directory while the first buffer is alive
/// must fail fast with [`SegmentError::Locked`].
#[test]
fn flock_second_open_returns_locked_error() {
    let tmp = TempDir::new().unwrap();
    let _first = test_buffer(tmp.path());
    let result = SegmentBuffer::<TestItem>::open(tmp.path(), test_config(1024 * 1024));
    let err = result.expect_err("second open must fail");
    assert!(
        matches!(err, SegmentError::Locked { .. }),
        "expected SegmentError::Locked, got {err:?}"
    );
    let rendered = format!("{err}");
    assert!(
        rendered.contains("locked by another process"),
        "error should mention lock: got {rendered}"
    );
}

/// After the first buffer is dropped, the lock releases and a new `open()`
/// succeeds. This is the kernel-advisory-lock contract: dropping the fd
/// releases the flock.
#[test]
fn flock_open_after_drop_succeeds() {
    let tmp = TempDir::new().unwrap();
    {
        let _first = test_buffer(tmp.path());
        assert!(SegmentBuffer::<TestItem>::open(tmp.path(), test_config(1024 * 1024)).is_err());
        // _first dropped here: the flock is released.
    }
    let _second = test_buffer(tmp.path());
    // If we reached here without panicking, the second open succeeded.
}

/// A lock file is created in the directory as a side-effect of `open()`.
/// Operators can list it; recovery must ignore it (it does not match
/// `seg_*_*.zst`).
#[test]
fn flock_creates_lock_sidecar_file() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    // Append + flush so a segment file exists alongside the lock file.
    buf.append(test_item(0)).unwrap();
    buf.flush().unwrap();

    let entries: Vec<String> = std::fs::read_dir(tmp.path())
        .expect("dir readable")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.iter().any(|n| n == ".segment-buffer.lock"),
        "lock sidecar must exist; dir contents: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|n| n.starts_with("seg_") && n.ends_with(".zst")),
        "segment file must exist; dir contents: {entries:?}"
    );
    // The lock file must not be confused with a segment: scan_segments returns
    // exactly one segment (the lock is ignored).
    assert_eq!(
        buf.read_from(0, 100).unwrap().len(),
        1,
        "lock file must not show up as a segment"
    );
}

/// Different directories are independently lockable. Two buffers in two
/// directories must both succeed.
#[test]
fn flock_locks_are_per_directory() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let _a = test_buffer(tmp_a.path());
    let _b = test_buffer(tmp_b.path());
    // No panic — both opens succeeded.
}

// =========================================================================
// SegmentIter (M7)
// =========================================================================

#[test]
fn iter_from_yields_seq_item_pairs() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    // Batch(4) triggers a flush on the 4th append; one item stays in memory.
    let collected: Vec<(u64, TestItem)> = buf.iter_from(0, 100).unwrap().collect();
    assert_eq!(
        collected.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        collected.iter().map(|(_, i)| i.id).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
}

#[test]
fn iter_from_limit_zero_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();
    let count = buf.iter_from(0, 0).unwrap().count();
    assert_eq!(count, 0);
}

#[test]
fn iter_from_respects_limit() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    let collected: Vec<u64> = buf.iter_from(2, 3).unwrap().map(|(_, i)| i.id).collect();
    assert_eq!(collected, vec![2, 3, 4]);
}

#[test]
fn iter_from_chains_with_iterator_combinators() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // The classic Iterator combinators all work.
    let sum: u64 = buf
        .iter_from(0, 100)
        .unwrap()
        .map(|(_, i)| i.id)
        .filter(|x| x % 2 == 0)
        .sum();
    assert_eq!(sum, 2 + 4 + 6 + 8);
}

#[test]
fn iter_from_start_seq_skips_already_read_items() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..5 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    let collected: Vec<u64> = buf.iter_from(3, 100).unwrap().map(|(_, i)| i.id).collect();
    assert_eq!(collected, vec![3, 4]);
}

// =========================================================================
// mtime probe for scan cache (M13)
// =========================================================================

/// On a real filesystem (the tempdir), the probe should return `true` —
/// mtime moves when we write twice with a sleep in between. On a
/// filesystem that pins mtime to a constant, the probe returns `false`
/// and the cache guard is skipped (today's behavior).
#[test]
fn mtime_probe_returns_true_on_real_filesystem() {
    let tmp = TempDir::new().unwrap();
    // The probe runs at open(). Today's CI is on real filesystems
    // (ext4/tmpfs on Linux, apfs on macOS) where mtime is fine-grained.
    let buf = test_buffer(tmp.path());
    assert!(
        buf.mtime_supported,
        "mtime capability probe should return true on the host filesystem; \
         if this fires, the test host has a coarse-granularity or no-mtime \
         filesystem (the cache guard is correctly disabled in that case, \
         but the test assertion needs to match)"
    );
}

/// External directory mutation must be detected by the mtime guard: if
/// someone removes a segment file out from under us, the next `scan_cache`
/// hit must NOT serve the stale list (would silently drop the segment's
/// items from reads).
#[test]
fn external_segment_removal_invalidates_scan_cache() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // Two flushes so we have two segments to reason about.
    for i in 4..8 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    // 8 items readable through the public API.
    assert_eq!(buf.read_from(0, 100).unwrap().len(), 8);

    // Simulate an external process quarantining one segment.
    let segments: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("dir readable")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "zst"))
        .collect();
    assert_eq!(segments.len(), 2, "expected exactly two segments on disk");
    let _ = std::fs::remove_file(&segments[0]);

    // Sleep briefly so the dir mtime moves past the cached value (the
    // probe sleep is 15ms; we use 25ms here for headroom on coarse fs).
    std::thread::sleep(std::time::Duration::from_millis(25));

    // The next read must observe the removal: only one segment's items
    // (4) survive. Without the mtime guard, the stale cache would still
    // report both segments as on-disk and reads would try (and fail) to
    // open the removed file — surfacing as an Err.
    let after = buf.read_from(0, 100).unwrap().len();
    assert_eq!(
        after, 4,
        "external removal must be reflected via mtime guard"
    );
}

// =========================================================================
// Debug impl for SegmentBuffer<T>
// =========================================================================

#[test]
fn debug_impl_formats_cleanly() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();
    buf.append(test_item(1)).unwrap();
    buf.append(test_item(2)).unwrap();

    let rendered = format!("{buf:?}");
    // Structural sanity: struct name + path field + every BufferStats field.
    assert!(
        rendered.starts_with("SegmentBuffer {"),
        "expected SegmentBuffer struct prefix, got: {rendered}"
    );
    // debug_struct renders field names as bare identifiers (no quotes).
    assert!(
        rendered.contains("dir: "),
        "Debug must expose the dir field, got: {rendered}"
    );
    for field in [
        "pending_count",
        "latest_sequence",
        "head_sequence",
        "next_sequence",
        "approx_disk_bytes",
        "segment_count",
        "max_size_bytes",
        "store_pressure",
    ] {
        assert!(
            rendered.contains(&format!("{field}: ")),
            "Debug must expose the `{field}` field, got: {rendered}"
        );
    }
    // pending_count reflects the three appends.
    assert!(
        rendered.contains("pending_count: 3"),
        "expected pending_count: 3, got: {rendered}"
    );
}

// =========================================================================
// Display snapshot tests — lock the format strings so a careless edit
// (e.g. changing a brace in a `thiserror` attribute) shows up as a test
// failure instead of silently shifting operator-facing log output.
// =========================================================================

#[test]
fn segment_error_io_display_format_no_path() {
    // Io constructed from a bare io::Error via `?` has site = Unknown.
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let err: SegmentError = io_err.into();
    let rendered = format!("{err}");
    // No " for ..." clause when site is Unknown.
    assert_eq!(rendered, "I/O error: missing");
}

#[test]
fn segment_error_io_display_format_with_segment_path() {
    // Io constructed with explicit Segment site renders the path clause.
    let err = SegmentError::Io {
        site: IoSite::Segment(std::path::PathBuf::from(
            "/var/data/seg_000000000000_000000000000.zst",
        )),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "I/O error for /var/data/seg_000000000000_000000000000.zst: permission denied"
    );
}

#[test]
fn segment_error_io_display_format_with_dir_site() {
    // Io with site = Dir renders a fixed clause (no path payload — the
    // directory is reachable via SegmentBuffer::path()).
    let err = SegmentError::Io {
        site: IoSite::Dir,
        source: std::io::Error::new(std::io::ErrorKind::ReadOnlyFilesystem, "read-only"),
    };
    let rendered = format!("{err}");
    assert_eq!(rendered, "I/O error for the segment directory: read-only");
}

#[test]
fn segment_error_with_path_upgrades_unknown_to_segment() {
    // with_path on an Unknown Io error upgrades the site to Segment.
    let raw: SegmentError = std::io::Error::other("boom").into();
    let upgraded = raw.with_path("/tmp/seg.zst");
    match upgraded {
        SegmentError::Io {
            site: IoSite::Segment(p),
            ..
        } => {
            assert_eq!(p, std::path::PathBuf::from("/tmp/seg.zst"));
        }
        other => panic!("expected Io with Segment site, got {other:?}"),
    }
}

#[test]
fn segment_error_with_path_leaves_segment_alone() {
    // First call site to attach context wins: calling with_path on a Segment
    // site leaves the original path intact (no clobbering).
    let err = SegmentError::Io {
        site: IoSite::Segment(std::path::PathBuf::from("/original/path.zst")),
        source: std::io::Error::other("x"),
    };
    let upgraded = err.with_path("/wrong/attempt.zst");
    match upgraded {
        SegmentError::Io {
            site: IoSite::Segment(p),
            ..
        } => {
            assert_eq!(p, std::path::PathBuf::from("/original/path.zst"));
        }
        other => panic!("expected Io with original Segment site, got {other:?}"),
    }
}

#[test]
fn segment_error_with_dir_upgrades_unknown_to_dir() {
    // with_dir on an Unknown Io error tags the site as Dir.
    let raw: SegmentError = std::io::Error::other("boom").into();
    let tagged = raw.with_dir();
    assert!(matches!(
        tagged,
        SegmentError::Io {
            site: IoSite::Dir,
            ..
        }
    ));
}

#[test]
fn segment_error_io_with_path_attaches_path() {
    // Upgrade a bare propagated io::Error to carry path context.
    let io_err: SegmentError =
        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read").into();
    let upgraded = io_err.with_path("/tmp/seg_000000000000_000000000000.zst");
    let rendered = format!("{upgraded}");
    assert_eq!(
        rendered,
        "I/O error for /tmp/seg_000000000000_000000000000.zst: short read"
    );
}

#[test]
fn segment_error_cbor_display_format() {
    let err = SegmentError::Cbor {
        phase: "deserialize",
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        message: "unexpected eof".into(),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "CBOR deserialize failed for /var/data/seg_000000000000_000000000000.zst: unexpected eof"
    );
}

#[test]
fn segment_error_cipher_display_format() {
    let err = SegmentError::Cipher {
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        message: "AES-GCM decryption failed".into(),
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "cipher error for /var/data/seg_000000000000_000000000000.zst: AES-GCM decryption failed"
    );
}

#[test]
fn segment_error_integrity_display_format() {
    let err = SegmentError::Integrity {
        path: std::path::PathBuf::from("/var/data/seg_000000000000_000000000000.zst"),
        reason: "truncated payload",
    };
    let rendered = format!("{err}");
    assert_eq!(
        rendered,
        "integrity failure for /var/data/seg_000000000000_000000000000.zst: truncated payload"
    );
}

#[test]
fn cipher_error_msg_display_format() {
    let err = super::CipherError::msg("key not configured");
    let rendered = format!("{err}");
    // msg() preserves the message verbatim; no prefix or decoration.
    assert_eq!(rendered, "key not configured");
}

#[test]
#[cfg(feature = "encryption")]
fn cipher_error_with_source_display_format() {
    use std::error::Error as _;

    #[derive(Debug)]
    struct FakeAead;
    impl std::fmt::Display for FakeAead {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("aead tag mismatch")
        }
    }
    impl std::error::Error for FakeAead {}

    let err = super::CipherError::with_source("AES-GCM decryption failed", FakeAead);
    // Display intentionally hides the source chain — the message stands alone.
    // The underlying cause is reachable only via `Error::source()`.
    assert_eq!(format!("{err}"), "AES-GCM decryption failed");
    let src = err.source().expect("with_source must populate source()");
    assert_eq!(format!("{src}"), "aead tag mismatch");
}

// =========================================================================
// for_each_from re-entrancy safety (panic-free, no deadlock)
// =========================================================================

#[test]
fn for_each_from_allows_reentry_without_deadlock() {
    // The buffer mutex is never held across a for_each_from callback: on-disk
    // items are decoded before the callback and in-memory items are snapshotted
    // under the lock then released. Re-entrant read calls must therefore
    // succeed instead of panicking or deadlocking.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));
    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }

    let buf_clone = Arc::clone(&buf);
    let mut counts_during_iteration = Vec::new();
    let visited = buf
        .for_each_from(0, 100, |_seq, _item| {
            counts_during_iteration.push(buf_clone.pending_count());
            let _ = buf_clone.latest_sequence();
            let _ = buf_clone.stats();
        })
        .unwrap();

    assert_eq!(visited, 3);
    // pending_count stays 3 throughout (no concurrent mutation).
    assert!(
        counts_during_iteration.iter().all(|&c| c == 3),
        "re-entrant reads must succeed and stay consistent: {counts_during_iteration:?}"
    );
}

#[test]
fn for_each_from_allows_reentrant_mutation() {
    // Mutating re-entrant calls (append) must also be safe: they acquire the
    // mutex normally because for_each_from released it before the callback.
    // Manual flush keeps the assertions deterministic (no auto-flush side
    // effects). The snapshot taken before the callbacks is unaffected by the
    // re-entrant appends, so the visited count stays at the original 3.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }

    let buf_clone = Arc::clone(&buf);
    let visited = buf
        .for_each_from(0, 3, |_seq, _item| {
            let _ = buf_clone.append(test_item(99));
        })
        .unwrap();

    assert_eq!(visited, 3, "snapshot taken before callbacks is unaffected");
    // Three re-entrant appends landed in the in-memory tail (Manual = no flush).
    assert_eq!(buf.pending_count(), 6);
    assert_eq!(buf.latest_sequence(), 5);
}

#[test]
fn for_each_from_usable_after_panicking_callback() {
    // A panicking callback must not brick the buffer. The mutex is never held
    // across the callback, so unwinding leaves the buffer consistent.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));
    for i in 0..3 {
        buf.append(test_item(i)).unwrap();
    }

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = buf.for_each_from(0, 100, |_seq, _item| {
            panic!("boom inside callback");
        });
    }));

    assert_eq!(buf.pending_count(), 3, "buffer must be usable after panic");
    assert_eq!(buf.latest_sequence(), 2);
}

// =========================================================================
// append_all batch primitive
// =========================================================================

#[test]
fn append_all_assigns_contiguous_sequences() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());

    let last = buf
        .append_all([test_item(1), test_item(2), test_item(3)])
        .unwrap();
    assert_eq!(last, 2, "last seq should be 2 (0-based)");
    assert_eq!(buf.pending_count(), 3);

    // A second batch continues the sequence.
    let last2 = buf.append_all([test_item(4), test_item(5)]).unwrap();
    assert_eq!(last2, 4);
    assert_eq!(buf.pending_count(), 5);
}

#[test]
fn append_all_empty_iterator_is_noop() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();

    let last = buf.append_all(std::iter::empty::<TestItem>()).unwrap();
    assert_eq!(last, 0, "empty append_all returns current last seq");
    assert_eq!(buf.pending_count(), 1);
}

#[test]
fn append_all_visibly_cheaper_lock_count_than_loop_append() {
    // Not a perf test — a correctness test: append_all assigns contiguous
    // seqs even under concurrent writers, because the whole batch is under
    // one lock. Two concurrent append_all calls must not interleave seqs.
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(test_buffer(tmp.path()));

    thread::scope(|s| {
        let b1 = Arc::clone(&buf);
        s.spawn(move || {
            b1.append_all((0..100).map(test_item)).unwrap();
        });
        let b2 = Arc::clone(&buf);
        s.spawn(move || {
            b2.append_all((0..100).map(test_item)).unwrap();
        });
    });

    // All 200 items must be present. Seqs are contiguous but the two batches
    // may land in either order.
    assert_eq!(buf.pending_count(), 200);
    assert_eq!(buf.latest_sequence(), 199);
}

#[test]
fn append_all_auto_flush_increments_segment_count() {
    // append_all routes through the same flush() (and therefore the same
    // segment_count fetch_add(1)) codepath as single-append auto-flush when
    // the batch threshold is crossed. Assert the live segment_count reflects
    // the auto-flushed segment — closes the gap where append_all-triggered
    // auto-flush had no explicit segment_count assertion.
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path()); // FlushPolicy::Batch(4)

    // Below threshold: no auto-flush, segment_count stays 0.
    buf.append_all([test_item(0), test_item(1), test_item(2)])
        .unwrap();
    assert_eq!(
        buf.stats().segment_count,
        0,
        "below batch threshold, append_all must not auto-flush"
    );

    // Crossing the threshold (4 items pending) triggers an auto-flush inside
    // append_all, writing all pending items as exactly one segment.
    buf.append_all([test_item(3), test_item(4), test_item(5), test_item(6)])
        .unwrap();
    assert_eq!(
        buf.stats().segment_count,
        1,
        "append_all crossing the batch threshold must auto-flush exactly one segment"
    );
}

// =========================================================================
// path() and config() accessors
// =========================================================================

#[test]
fn path_accessor_returns_directory() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    assert_eq!(buf.path(), tmp.path());
}

#[test]
fn config_accessor_returns_opened_config() {
    let tmp = TempDir::new().unwrap();
    let config = SegmentConfig {
        flush_policy: FlushPolicy::Batch(7),
        max_size_bytes: 42,
        compression_level: 9,
        durability: DurabilityPolicy::Throughput,
        cipher: None,
    };
    let buf = test_buffer_with_config(tmp.path(), config);
    let cfg = buf.config();
    assert_eq!(cfg.flush_policy, FlushPolicy::Batch(7));
    assert_eq!(cfg.max_size_bytes, 42);
    assert_eq!(cfg.compression_level, 9);
    assert_eq!(cfg.durability, DurabilityPolicy::Throughput);
}

fn test_buffer_with_config(dir: &Path, config: SegmentConfig) -> TestBuffer {
    SegmentBuffer::open(dir, config).expect("buffer must open")
}

// =========================================================================
// sync_disk_bytes
// =========================================================================

#[test]
fn sync_disk_bytes_recovers_after_external_truncation() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = test_buffer(dir);
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();

    let before = buf.stats().approx_disk_bytes;
    assert!(before > 0, "flushed segment should have nonzero size");

    // External process truncates all segment files to zero bytes.
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "zst") {
            fs::write(&path, b"").unwrap();
        }
    }

    let synced = buf.sync_disk_bytes().unwrap();
    assert_eq!(
        synced, 0,
        "external truncation must be reflected after sync"
    );
    assert_eq!(buf.stats().approx_disk_bytes, 0);
}

// =========================================================================
// Live segment_count in BufferStats — tracks the on-disk segment file count
// incrementally alongside approx_disk_bytes.
// =========================================================================

/// Helper: count `.zst` segment files actually on disk.
fn count_disk_segments(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
        .count() as u64
}

#[test]
fn segment_count_zero_on_fresh_buffer() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    assert_eq!(
        buf.stats().segment_count,
        0,
        "fresh buffer must report 0 segments"
    );
}

#[test]
fn segment_count_increments_on_flush() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    assert_eq!(
        buf.stats().segment_count,
        1,
        "one flush must produce exactly one segment in the live count"
    );
    assert_eq!(
        count_disk_segments(tmp.path()),
        1,
        "live count must match actual disk file count"
    );
}

#[test]
fn segment_count_tracks_multiple_flushes() {
    let tmp = TempDir::new().unwrap();
    let config = SegmentConfig {
        flush_policy: FlushPolicy::Manual,
        ..test_config(1024 * 1024)
    };
    let buf = SegmentBuffer::open(tmp.path(), config).unwrap();

    for round in 1..=5u64 {
        for i in 0..3u64 {
            buf.append(test_item(round * 10 + i)).unwrap();
        }
        buf.flush().unwrap();
        assert_eq!(
            buf.stats().segment_count,
            round,
            "after {round} flushes the live segment_count must be {round}"
        );
        assert_eq!(
            count_disk_segments(tmp.path()),
            round,
            "live count must match disk after {round} flushes"
        );
    }
}

#[test]
fn segment_count_decrements_on_delete_acked() {
    let tmp = TempDir::new().unwrap();
    let config = SegmentConfig {
        flush_policy: FlushPolicy::Manual,
        ..test_config(1024 * 1024)
    };
    let buf = SegmentBuffer::open(tmp.path(), config).unwrap();

    // Three independent segments: [0..=0], [1..=1], [2..=2].
    buf.append(test_item(0)).unwrap();
    buf.flush().unwrap();
    buf.append(test_item(1)).unwrap();
    buf.flush().unwrap();
    buf.append(test_item(2)).unwrap();
    buf.flush().unwrap();
    assert_eq!(buf.stats().segment_count, 3);

    // Ack seq 0 → removes one segment.
    let removed = buf.delete_acked(0).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(buf.stats().segment_count, 2);
    assert_eq!(count_disk_segments(tmp.path()), 2);

    // Ack seq 2 → removes both remaining.
    let removed = buf.delete_acked(2).unwrap();
    assert_eq!(removed, 2);
    assert_eq!(buf.stats().segment_count, 0);
    assert_eq!(count_disk_segments(tmp.path()), 0);
}

#[test]
fn segment_count_recalibrated_by_sync_disk_bytes() {
    let tmp = TempDir::new().unwrap();
    let buf = test_buffer(tmp.path());
    buf.append(test_item(0)).unwrap();
    buf.append(test_item(1)).unwrap();
    buf.flush().unwrap();
    assert_eq!(buf.stats().segment_count, 1);

    // Simulate an external process removing the segment file behind the buffer.
    for entry in fs::read_dir(tmp.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "zst") {
            fs::remove_file(&path).unwrap();
        }
    }

    let _synced = buf.sync_disk_bytes().unwrap();
    assert_eq!(
        buf.stats().segment_count,
        0,
        "sync_disk_bytes must recalibrate segment_count to the directory reality"
    );
}

#[test]
fn segment_count_recovered_on_reopen() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // First instance: write and flush two segments.
    {
        let config = SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        };
        let buf: TestBuffer = SegmentBuffer::open(dir, config).unwrap();
        buf.append(test_item(0)).unwrap();
        buf.flush().unwrap();
        buf.append(test_item(1)).unwrap();
        buf.flush().unwrap();
        assert_eq!(buf.stats().segment_count, 2);
    }

    // Re-open: recovery must restore the live segment_count.
    let (buf, report) =
        SegmentBuffer::<TestItem>::open_with_report(dir, test_config(1024 * 1024)).unwrap();
    assert_eq!(report.segment_count, 2, "recovery report snapshot");
    assert_eq!(
        buf.stats().segment_count,
        2,
        "live segment_count must match recovery report right after open"
    );
}

#[test]
fn segment_count_stress_4_writers_2_deleters() {
    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );

    // Pre-seed a few segments so deleters have something to remove while
    // writers are still producing new ones.
    {
        let b = Arc::clone(&buf);
        for round in 0..3u64 {
            for j in 0..4u64 {
                b.append(test_item(round * 10 + j)).unwrap();
            }
            b.flush().unwrap();
        }
    }

    std::thread::scope(|s| {
        for writer_id in 0..4usize {
            let b = Arc::clone(&buf);
            s.spawn(move || {
                for round in 0..25 {
                    for j in 0..4u64 {
                        let id = 100u64 + writer_id as u64 * 100 + round as u64 * 4 + j;
                        b.append(test_item(id)).unwrap();
                    }
                    b.flush().unwrap();
                }
            });
        }

        for _ in 0..2usize {
            let b = Arc::clone(&buf);
            s.spawn(move || {
                for _ in 0..20 {
                    let stats = b.stats();
                    if stats.next_sequence > stats.head_sequence && stats.pending_count == 0 {
                        let _ = b.delete_acked(stats.latest_sequence);
                    }
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            });
        }
    });

    // sync_disk_bytes recalibrates the live counters to the directory reality.
    buf.sync_disk_bytes().unwrap();
    let stats = buf.stats();
    assert_eq!(
        stats.segment_count,
        count_disk_segments(tmp.path()),
        "live segment_count must converge to the actual disk segment count"
    );
}

// under contention. Verifies correctness (all items readable) AND reports
// a throughput number so perf regressions show up in test output.
// =========================================================================

#[test]
fn stress_8_writers_2_readers_throughput() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    let tmp = TempDir::new().unwrap();
    // FlushPolicy::Manual is critical: with Batch(4) this test would create
    // 20_000 segment files (80_000 items / 4), causing pathological I/O under
    // parallel test execution. Manual keeps everything in-memory so the test
    // stresses mutex contention, not the filesystem. The single flush() after
    // the scope writes one segment.
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 8;
    const PER_WRITER: usize = 10_000;
    const TOTAL: usize = WRITERS * PER_WRITER; // 80_000
    const READERS: usize = 2;

    // Shared read cursor — readers use it as a hint for where to poll.
    // The cursor may drift ahead (double-reads are harmless); correctness is
    // verified by the final full read, not by the cursor value.
    let read_cursor = Arc::new(Mutex::new(0u64));
    let total_read = Arc::new(AtomicU64::new(0));

    let start = Instant::now();
    thread::scope(|s| {
        // 2 reader threads: poll read_from to add read-side contention.
        for _ in 0..READERS {
            let buf_r = Arc::clone(&buf);
            let cursor_r = Arc::clone(&read_cursor);
            let total_r = Arc::clone(&total_read);
            s.spawn(move || loop {
                let current = *cursor_r.lock();
                if current >= TOTAL as u64 {
                    break;
                }
                if let Ok(events) = buf_r.read_from(current, 500) {
                    if !events.is_empty() {
                        total_r.fetch_add(events.len() as u64, Ordering::Relaxed);
                        *cursor_r.lock() = current + events.len() as u64;
                    }
                }
                std::thread::sleep(Duration::from_micros(20));
            });
        }

        // 8 writer threads.
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            s.spawn(move || {
                let base = writer_id * PER_WRITER;
                for i in 0..PER_WRITER {
                    let _ = buf_w.append(test_item((base + i) as u64));
                }
            });
        }
    });

    let elapsed = start.elapsed();

    // Regression guard (AGENTS.md rule 7): under FlushPolicy::Manual the
    // concurrent append phase must create ZERO segment files. An earlier
    // Batch(4) version created 20_000 files and hung CI for hours (commit
    // 80257a0). If this fires, the flush policy or Manual semantics broke —
    // do NOT widen the bound, investigate the regression.
    let segment_files_before_flush = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        segment_files_before_flush, 0,
        "FlushPolicy::Manual must not create segment files during append; \
         found {segment_files_before_flush} .zst file(s) — flush policy regression"
    );

    buf.flush().unwrap();

    // Correctness: all items assigned and readable.
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);
    assert_eq!(buf.pending_count(), TOTAL as u64);
    let all_events = buf.read_from(0, TOTAL * 2).unwrap();
    assert_eq!(
        all_events.len(),
        TOTAL,
        "all {TOTAL} events must be readable after the stress run"
    );

    // Throughput: report events/sec. NOT a hard assertion (CI hardware varies)
    // — it's a reporting metric so a human can spot regressions in the test
    // output.
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let throughput = TOTAL as f64 / elapsed_secs;
    eprintln!(
        "stress_8w_2r: {TOTAL} events in {elapsed_secs:.3}s = {throughput:.0} events/sec \
         ({:.2} µs/event under 8-writer contention, {} items observed by readers)",
        elapsed_secs * 1_000_000.0 / TOTAL as f64,
        total_read.load(Ordering::Relaxed)
    );
}

/// 8 writers × 4 readers stress with per-append latency histogram.
///
/// Reports p50/p90/p99/p99.9 latency on the writer path so a human can spot
/// latency-tail regressions (e.g. a lock-contention change, an allocation
/// introduced on the hot path). The latency numbers are NOT hard assertions
/// (CI hardware varies) — they are reported in the test output for
/// human inspection across runs.
///
/// Reuses the rule-7 discipline from the throughput stress: `FlushPolicy::Manual`
/// so the test stresses mutex contention, not the filesystem. Reader count
/// is doubled (4 vs the throughput test's 2) so read-side contention
/// contributes to the writer-tail latency.
#[test]
fn stress_8_writers_4_readers_latency_histogram() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    let tmp = TempDir::new().unwrap();
    let buf = Arc::new(
        SegmentBuffer::open(
            tmp.path(),
            SegmentConfig {
                flush_policy: FlushPolicy::Manual,
                ..test_config(1024 * 1024)
            },
        )
        .unwrap(),
    );
    const WRITERS: usize = 8;
    const PER_WRITER: usize = 10_000;
    const TOTAL: usize = WRITERS * PER_WRITER; // 80_000
    const READERS: usize = 4;

    let read_cursor = Arc::new(Mutex::new(0u64));
    let total_read = Arc::new(AtomicU64::new(0));
    // Per-writer latency samples. Pre-allocate so sampling overhead doesn't
    // include allocation in the measured section.
    let samples: Vec<Mutex<Vec<std::time::Duration>>> = (0..WRITERS)
        .map(|_| Mutex::new(Vec::with_capacity(PER_WRITER)))
        .collect();

    let start = Instant::now();
    thread::scope(|s| {
        // 4 reader threads: poll read_from to add read-side contention.
        for _ in 0..READERS {
            let buf_r = Arc::clone(&buf);
            let cursor_r = Arc::clone(&read_cursor);
            let total_r = Arc::clone(&total_read);
            s.spawn(move || loop {
                let current = *cursor_r.lock();
                if current >= TOTAL as u64 {
                    break;
                }
                if let Ok(events) = buf_r.read_from(current, 500) {
                    if !events.is_empty() {
                        total_r.fetch_add(events.len() as u64, Ordering::Relaxed);
                        *cursor_r.lock() = current + events.len() as u64;
                    }
                }
                std::thread::sleep(Duration::from_micros(20));
            });
        }

        // 8 writer threads, each measuring per-append latency.
        for writer_id in 0..WRITERS {
            let buf_w = Arc::clone(&buf);
            let samples_w = &samples;
            s.spawn(move || {
                let base = writer_id * PER_WRITER;
                let mut local_samples: Vec<std::time::Duration> = Vec::with_capacity(PER_WRITER);
                for i in 0..PER_WRITER {
                    let t = Instant::now();
                    let _ = buf_w.append(test_item((base + i) as u64));
                    local_samples.push(t.elapsed());
                }
                *samples_w[writer_id].lock() = local_samples;
            });
        }
    });

    let elapsed = start.elapsed();

    // Regression guard (rule 7): under Manual the concurrent append phase
    // must create ZERO segment files.
    let segment_files_before_flush = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        segment_files_before_flush, 0,
        "FlushPolicy::Manual must not create segment files during append; \
         found {segment_files_before_flush} .zst file(s) — flush policy regression"
    );

    buf.flush().unwrap();
    assert_eq!(buf.latest_sequence(), (TOTAL - 1) as u64);

    // Merge per-writer samples into a single sorted Vec for percentile
    // computation. N = 80_000 samples is plenty for stable p99 estimates.
    let mut all: Vec<std::time::Duration> = Vec::with_capacity(TOTAL);
    for s in &samples {
        all.extend_from_slice(&s.lock());
    }
    all.sort();

    // Percentile helper. N is large enough that linear indexing is fine.
    let pct = |p: f64| -> std::time::Duration {
        if all.is_empty() {
            return std::time::Duration::ZERO;
        }
        let idx = ((p / 100.0) * (all.len() as f64 - 1.0)).round() as usize;
        all[idx.min(all.len() - 1)]
    };
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let throughput = TOTAL as f64 / elapsed_secs;
    eprintln!(
        "stress_8w_4r_latency: {TOTAL} events in {elapsed_secs:.3}s = {throughput:.0} events/sec\n\
         latency (µs): p50={:.2} p90={:.2} p99={:.2} p99.9={:.2} max={:.2}\n\
         {} items observed by readers",
        pct(50.0).as_nanos() as f64 / 1000.0,
        pct(90.0).as_nanos() as f64 / 1000.0,
        pct(99.0).as_nanos() as f64 / 1000.0,
        pct(99.9).as_nanos() as f64 / 1000.0,
        all.last().map_or(0.0, |d| d.as_nanos() as f64 / 1000.0),
        total_read.load(Ordering::Relaxed)
    );

    // Soft guard: p99 must stay under 5ms on any reasonable host (the test
    // runs in debug mode by default; release numbers are ~10x lower). This
    // is NOT a tight bound; if it fires, investigate the hot-path regression
    // before widening. Typical debug-mode p99 is ~50-500µs under 8-writer
    // contention.
    let p99 = pct(99.0);
    assert!(
        p99 < std::time::Duration::from_millis(50),
        "p99 latency {p99:?} exceeded 50ms soft guard — investigate hot-path regression"
    );
}

// =========================================================================
// DurabilityPolicy
// =========================================================================

/// Config-builder helper: vary ONLY the durability policy, keeping the
/// `test_config` defaults for everything else.
fn durability_config(max_size_bytes: u64, policy: DurabilityPolicy) -> SegmentConfig {
    SegmentConfig {
        flush_policy: FlushPolicy::Manual,
        max_size_bytes,
        compression_level: 3,
        durability: policy,
        cipher: None,
    }
}

/// All three policies must produce a readable, correct segment. This is a
/// functional roundtrip test, NOT a crash-semantics test — proving the
/// fsync branches fire correctly under a host crash requires killing the
/// process mid-flush and is out of scope for unit tests. (The fsync calls
/// are also exercised here: if a `sync_all` path is broken on the host, this
/// test surfaces it as an Err.)
#[test]
fn durability_policy_segment_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Segment),
    )
    .expect("open with Segment policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().expect("Segment flush must succeed");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }
}

#[test]
fn durability_policy_throughput_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Throughput),
    )
    .expect("open with Throughput policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush()
        .expect("Throughput flush must succeed (no fsync, but rename is real)");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }
}

#[test]
fn durability_policy_maximal_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::<TestItem>::open(
        tmp.path(),
        durability_config(1024 * 1024, DurabilityPolicy::Maximal),
    )
    .expect("open with Maximal policy");

    for i in 0..10 {
        buf.append(test_item(i)).unwrap();
    }
    // Maximal includes a dir.sync_all after rename; on Linux/macOS this is
    // well-defined and must succeed. If it errors here, the host filesystem
    // does not support directory fsync (Maximal is documented to require
    // Linux/macOS for the dir-sync half).
    buf.flush()
        .expect("Maximal flush must succeed on a capable filesystem");

    let items = buf.read_from(0, 100).unwrap();
    assert_eq!(items.len(), 10);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.id, i as u64);
    }

    // The directory must contain exactly one segment file (no .tmp debris
    // left behind) — verifies the rename completed under every policy.
    let zst_count = std::fs::read_dir(tmp.path())
        .expect("temp dir readable")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "zst"))
        .count();
    assert_eq!(
        zst_count, 1,
        "exactly one .zst segment must exist after flush"
    );
}

/// All three policies must be recoverable: re-open the directory and the
/// segment file is visible (the rename is the atomicity boundary under
/// every policy).
#[test]
fn durability_policy_all_policies_recover_after_reopen() {
    for policy in [
        DurabilityPolicy::Maximal,
        DurabilityPolicy::Segment,
        DurabilityPolicy::Throughput,
    ] {
        let tmp = TempDir::new().unwrap();
        {
            let buf =
                SegmentBuffer::<TestItem>::open(tmp.path(), durability_config(1024 * 1024, policy))
                    .expect("open");
            buf.append(test_item(42)).unwrap();
            buf.flush().expect("flush");
        }
        let (buf, report) = SegmentBuffer::<TestItem>::open_with_report(
            tmp.path(),
            durability_config(1024 * 1024, policy),
        )
        .expect("reopen");
        assert_eq!(
            report.segment_count, 1,
            "policy {policy:?}: segment must be recovered"
        );
        assert_eq!(report.head_seq, 0);
        assert_eq!(report.next_seq, 1);
        let items = buf.read_from(0, 100).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 42);
    }
}

/// The default `SegmentConfig` must select `Segment` (the documented
/// backward-compat default for one release after the enum lands).
#[test]
fn durability_policy_default_is_segment() {
    let cfg = SegmentConfig::default();
    assert_eq!(cfg.durability, DurabilityPolicy::Segment);
}

/// The builder `.durability(...)` setter must round-trip into the built config.
#[test]
fn durability_policy_builder_roundtrip() {
    let cfg = SegmentConfig::builder()
        .durability(DurabilityPolicy::Throughput)
        .build();
    assert_eq!(cfg.durability, DurabilityPolicy::Throughput);
}

/// `SegmentConfig` is `Clone` since the cipher moved from `Box` to `Arc`.
/// A roundtrip through `.clone()` must preserve every field, including the
/// cipher (the `Arc` is shared, not duplicated).
#[test]
fn segment_config_is_clone() {
    let cfg = SegmentConfig::default();
    let cloned = cfg.clone();
    assert_eq!(cfg.flush_policy, cloned.flush_policy);
    assert_eq!(cfg.max_size_bytes, cloned.max_size_bytes);
    assert_eq!(cfg.compression_level, cloned.compression_level);
    assert_eq!(cfg.durability, cloned.durability);
    assert!(cfg.cipher.is_none() && cloned.cipher.is_none());
}

/// `SegmentConfigBuilder` is `Clone` (M12). This unblocks the pattern of
/// starting a base builder and cloning it per-buffer when constructing
/// several related buffers (e.g. a sharded producer set). The cipher
/// `Arc` is shared between clones — no key duplication.
#[test]
fn segment_config_builder_is_clone() {
    let base = SegmentConfig::builder()
        .flush_manually()
        .compression_level(7);
    let copy = base.clone();
    let cfg_a = base.build();
    let cfg_b = copy.compression_level(1).build();
    assert_eq!(cfg_a.compression_level, 7);
    assert_eq!(cfg_b.compression_level, 1);
    assert!(matches!(cfg_a.flush_policy, FlushPolicy::Manual));
    assert!(matches!(cfg_b.flush_policy, FlushPolicy::Manual));
}

#[cfg(feature = "encryption")]
#[test]
fn segment_config_clone_shares_cipher_arc() {
    let cfg = SegmentConfig::builder()
        .cipher(Arc::new(AesGcmCipher::new(&[0u8; 32])))
        .build();
    let cloned = cfg.clone();
    // Both configs reference the SAME Arc — cipher state is shared, not
    // duplicated. This is what makes `recommended_cipher()` and multi-buffer
    // setups cheap.
    let (Some(a), Some(b)) = (cfg.cipher.as_ref(), cloned.cipher.as_ref()) else {
        panic!("cipher must be Some on both configs");
    };
    assert!(
        std::ptr::addr_eq(a.as_ref() as *const _, b.as_ref() as *const _),
        "Arc must be shared, not deep-copied"
    );
}

#[cfg(feature = "encryption")]
#[test]
fn aes_gcm_new_and_from_slice_are_equivalent() {
    // The infallible `new(&[u8; 32])` and fallible `from_slice(&[u8])`
    // constructors must produce ciphers with the same underlying key
    // material: encrypt with one, decrypt with the other.
    let key = [42u8; 32];
    let infallible = AesGcmCipher::new(&key);
    let fallible = AesGcmCipher::from_slice(&key).unwrap();

    let plaintext = b"cross-constructor roundtrip";
    let ct = infallible.encrypt(plaintext).unwrap();
    let pt = fallible.decrypt(&ct).unwrap();
    assert_eq!(pt.as_slice(), plaintext);

    let ct2 = fallible.encrypt(plaintext).unwrap();
    let pt2 = infallible.decrypt(&ct2).unwrap();
    assert_eq!(pt2.as_slice(), plaintext);
}

#[cfg(feature = "encryption")]
#[test]
fn xchacha20_new_and_from_slice_are_equivalent() {
    let key = [99u8; 32];
    let infallible = XChaCha20Poly1305Cipher::new(&key);
    let fallible = XChaCha20Poly1305Cipher::from_slice(&key).unwrap();

    let plaintext = b"cross-constructor roundtrip";
    let ct = infallible.encrypt(plaintext).unwrap();
    let pt = fallible.decrypt(&ct).unwrap();
    assert_eq!(pt.as_slice(), plaintext);

    let ct2 = fallible.encrypt(plaintext).unwrap();
    let pt2 = infallible.decrypt(&ct2).unwrap();
    assert_eq!(pt2.as_slice(), plaintext);
}

// =========================================================================
// segment_size_stats — on-demand min/max/mean/p50/p90 size distribution.
// =========================================================================

#[test]
fn segment_size_stats_all_zero_when_nothing_flushed() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        },
    )
    .unwrap();
    // Append without flushing: items stay in memory, no segment files on disk.
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    let s = buf.segment_size_stats().unwrap();
    assert_eq!(s.count, 0, "no segments on disk yet");
    assert_eq!(s.min_bytes, 0);
    assert_eq!(s.max_bytes, 0);
    assert_eq!(s.mean_bytes, 0);
    assert_eq!(s.p50_bytes, 0);
    assert_eq!(s.p90_bytes, 0);
}

#[test]
fn segment_size_stats_single_segment_all_fields_equal() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        },
    )
    .unwrap();
    for i in 0..4 {
        buf.append(test_item(i)).unwrap();
    }
    buf.flush().unwrap();
    let s = buf.segment_size_stats().unwrap();
    assert_eq!(s.count, 1);
    assert!(s.max_bytes > 0, "flushed segment must have nonzero size");
    // With one segment, min == mean == p50 == p90 == max.
    assert_eq!(s.min_bytes, s.max_bytes);
    assert_eq!(s.mean_bytes, s.max_bytes);
    assert_eq!(s.p50_bytes, s.max_bytes);
    assert_eq!(s.p90_bytes, s.max_bytes);
}

#[test]
fn segment_size_stats_matches_manual_recompute_and_percentiles() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        },
    )
    .unwrap();
    // Five segments of varying item counts so the byte sizes differ.
    for n in [3u64, 1, 5, 2, 4] {
        for i in 0..n {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }
    let s = buf.segment_size_stats().unwrap();
    assert_eq!(s.count, 5);
    assert_eq!(s.count, count_disk_segments(tmp.path()));

    // Independent brute-force straight from the directory.
    let mut sizes: Vec<u64> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
        .map(|e| e.metadata().map_or(0, |m| m.len()))
        .collect();
    sizes.sort();
    assert_eq!(s.min_bytes, *sizes.first().unwrap());
    assert_eq!(s.max_bytes, *sizes.last().unwrap());
    let total: u64 = sizes.iter().sum();
    assert_eq!(s.mean_bytes, total / sizes.len() as u64);
    // Cross-validate the nearest-rank percentiles against an independent
    // float implementation of ceil(p/100 · n).
    let n = sizes.len();
    let rank = |pct: f64| -> usize {
        let r = (pct / 100.0 * n as f64).ceil() as usize;
        r.clamp(1, n) - 1
    };
    assert_eq!(s.p50_bytes, sizes[rank(50.0)]);
    assert_eq!(s.p90_bytes, sizes[rank(90.0)]);
    // Monotonicity invariant.
    assert!(s.min_bytes <= s.p50_bytes);
    assert!(s.p50_bytes <= s.p90_bytes);
    assert!(s.p90_bytes <= s.max_bytes);
}

#[test]
fn segment_size_stats_reflects_delete_acked() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        },
    )
    .unwrap();
    // Three segments: [0,3], [4,7], [8,11] (4 items each).
    for _ in 0..3 {
        for i in 0..4u64 {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }
    let before = buf.segment_size_stats().unwrap();
    assert_eq!(before.count, 3);

    // Ack the first segment (covers seqs 0..=3).
    buf.delete_acked(3).unwrap();
    let after = buf.segment_size_stats().unwrap();
    assert_eq!(
        after.count, 2,
        "first segment must be gone after acking seq 3"
    );
    assert_eq!(after.count, count_disk_segments(tmp.path()));
    assert!(after.max_bytes <= before.max_bytes);
}

#[test]
fn segment_size_stats_count_and_mean_consistent_after_sync() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            ..test_config(1024 * 1024)
        },
    )
    .unwrap();
    for _ in 0..4 {
        for i in 0..5u64 {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }
    // Recalibrate the atomic counters, then compare scan-derived count.
    buf.sync_disk_bytes().unwrap();
    let s = buf.segment_size_stats().unwrap();
    let stats = buf.stats();
    assert_eq!(s.count, stats.segment_count);

    // mean = total / count (truncated), so mean·count <= total < mean·count + count.
    let actual_total: u64 = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
        .map(|e| e.metadata().map_or(0, |m| m.len()))
        .sum();
    let mean_times_count = s.mean_bytes.saturating_mul(s.count);
    assert!(
        mean_times_count <= actual_total,
        "mean·count must not exceed the real total"
    );
    assert!(
        actual_total < mean_times_count + s.count,
        "truncation error must be less than count"
    );
}

// =========================================================================
// percentile_of_sorted — direct edge-case tests for the private nearest-rank
// helper. Until now it was only exercised indirectly through segment_size_stats.
// These tests make the nearest-rank contract visible and pinned.
// =========================================================================

#[test]
fn percentile_of_sorted_empty_returns_zero() {
    let sorted: [u64; 0] = [];
    assert_eq!(
        SegmentBuffer::<TestItem>::percentile_of_sorted(&sorted, 50),
        0,
        "empty input must return 0"
    );
}

#[test]
fn percentile_of_sorted_pct_zero_returns_minimum() {
    // pct=0: rank = ceil(0/100 . n) = 0, clamped to 1, so first element.
    let sorted = [10u64, 20, 30, 40, 50];
    assert_eq!(
        SegmentBuffer::<TestItem>::percentile_of_sorted(&sorted, 0),
        10,
        "pct=0 must return the smallest element"
    );
}

#[test]
fn percentile_of_sorted_pct_hundred_returns_maximum() {
    // pct=100: rank = ceil(100/100 . n) = n, so last element.
    let sorted = [10u64, 20, 30, 40, 50];
    assert_eq!(
        SegmentBuffer::<TestItem>::percentile_of_sorted(&sorted, 100),
        50,
        "pct=100 must return the largest element"
    );
}

#[test]
fn percentile_of_sorted_single_element_returns_it_for_all_pct() {
    let sorted = [42u64];
    for pct in 0u32..=100 {
        assert_eq!(
            SegmentBuffer::<TestItem>::percentile_of_sorted(&sorted, pct),
            42,
            "single element must be returned for pct={pct}"
        );
    }
}

#[test]
fn percentile_of_sorted_is_monotonically_nondecreasing_in_pct() {
    let sorted = [5u64, 10, 15, 20, 25, 30, 35, 40, 45, 50];
    let mut prev = 0u64;
    for pct in 0u32..=100 {
        let val = SegmentBuffer::<TestItem>::percentile_of_sorted(&sorted, pct);
        assert!(
            val >= prev,
            "result must be non-decreasing: pct={pct} gave {val} < {prev}"
        );
        prev = val;
    }
}

#[cfg(feature = "encryption")]
#[test]
fn segment_size_stats_works_with_encrypted_segments() {
    let tmp = TempDir::new().unwrap();
    let buf = SegmentBuffer::open(
        tmp.path(),
        SegmentConfig {
            flush_policy: FlushPolicy::Manual,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            durability: DurabilityPolicy::Segment,
            cipher: Some(Arc::new(AesGcmCipher::new(&[0u8; 32]))),
        },
    )
    .unwrap();

    // Three flushes with varying item counts so byte sizes differ.
    for n in [3u64, 1, 5] {
        for i in 0..n {
            buf.append(test_item(i)).unwrap();
        }
        buf.flush().unwrap();
    }

    let s = buf.segment_size_stats().unwrap();
    assert_eq!(s.count, 3);
    assert_eq!(s.count, count_disk_segments(tmp.path()));

    // Cross-check every field against a brute-force directory scan.
    let mut sizes: Vec<u64> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
        .map(|e| e.metadata().map_or(0, |m| m.len()))
        .collect();
    sizes.sort();
    assert_eq!(s.min_bytes, *sizes.first().unwrap());
    assert_eq!(s.max_bytes, *sizes.last().unwrap());
    let total: u64 = sizes.iter().sum();
    assert_eq!(s.mean_bytes, total / sizes.len() as u64);
    let n = sizes.len();
    let rank = |pct: f64| -> usize {
        let r = (pct / 100.0 * n as f64).ceil() as usize;
        r.clamp(1, n) - 1
    };
    assert_eq!(s.p50_bytes, sizes[rank(50.0)]);
    assert_eq!(s.p90_bytes, sizes[rank(90.0)]);
    assert!(s.min_bytes <= s.p50_bytes);
    assert!(s.p50_bytes <= s.p90_bytes);
    assert!(s.p90_bytes <= s.max_bytes);
}
