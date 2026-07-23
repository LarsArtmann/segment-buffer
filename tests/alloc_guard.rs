//! Allocation-count regression guard.
//!
//! Measures heap allocation events on the hot paths (`append`, `read_from`,
//! `stats`) and asserts they stay within a fixed budget. This is a
//! machine-independent proxy for tail-latency regression: each allocation is
//! a potential page fault, syscall, or lock contention. The budget catches
//! regressions that add allocations (extra clones, Vec growth, `format!` in
//! hot loops) on every machine, without the hardware variance that makes
//! absolute µs thresholds flaky in CI.
//!
//! See `docs/perf/2026-07-23_percentile-latency-baseline.md` for the full
//! rationale and the relationship between allocation counts and p99 latency.

use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Counting allocator — intercepts every alloc / growing-realloc call.
// ---------------------------------------------------------------------------

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static TRACKING: AtomicBool = AtomicBool::new(false);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) && new_size > layout.size() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_allocs<F: FnOnce()>(f: F) -> usize {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
    std::sync::atomic::fence(Ordering::SeqCst);
    f();
    std::sync::atomic::fence(Ordering::SeqCst);
    TRACKING.store(false, Ordering::SeqCst);
    ALLOC_COUNT.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Test item — fixed-size to keep allocation counts stable across runs.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Item {
    id: u64,
    payload: [u8; 32],
}

fn item(n: u64) -> Item {
    Item {
        id: n,
        payload: [0; 32],
    }
}

fn open_buffer(tmp: &TempDir) -> SegmentBuffer<Item> {
    let mut config = SegmentConfig::default();
    config.flush_policy = FlushPolicy::Manual;
    config.max_size_bytes = 100 * 1024 * 1024;
    config.durability = DurabilityPolicy::Throughput;
    SegmentBuffer::open(tmp.path(), config).unwrap()
}

// ---------------------------------------------------------------------------
// Budgets — measured, then set with margin.
// ---------------------------------------------------------------------------

/// Maximum allocations for a warm `append` (Vec has capacity, no flush).
/// Measured: 0. Budget: 1 — allows minor allocator overhead on other platforms.
const WARM_APPEND_BUDGET: usize = 1;

/// Maximum allocations for `read_from(0, 50)` from in-memory items only
/// (no segment files). Measured: 1 (the result Vec). Budget: 3 — covers
/// the result Vec + margin for scan-cache overhead on other platforms.
const READ_FROM_50_INMEM_BUDGET: usize = 3;

/// Maximum allocations for `stats()`. Measured: 0. Budget: 1 — a
/// single-lock snapshot that should allocate almost nothing.
const STATS_BUDGET: usize = 1;

/// Maximum allocations for `append` that triggers a `flush` (encode +
/// compress + write one segment). Measured: 27. Budget: 32 — covers CBOR
/// encode Vec, zstd compress, file I/O buffers, with ~18% margin.
const APPEND_WITH_FLUSH_BUDGET: usize = 32;

#[test]
fn hot_path_allocation_budgets() {
    let tmp = TempDir::new().unwrap();
    let buf = open_buffer(&tmp);

    // Warm up: append enough items so `unflushed` has spare capacity.
    for i in 0..100u64 {
        buf.append(item(i)).unwrap();
    }

    // --- Warm append (no Vec growth, no flush) ---
    let warm_append = count_allocs(|| {
        buf.append(item(42)).unwrap();
    });
    assert!(
        warm_append <= WARM_APPEND_BUDGET,
        "warm append allocated {warm_append} events, budget {WARM_APPEND_BUDGET}; \
         a regression introduced extra allocations on the append hot path"
    );

    // --- read_from from in-memory items (Phase 2 only, no segments on disk) ---
    let warm_read = count_allocs(|| {
        let _items = buf.read_from(0, 50).unwrap();
    });
    assert!(
        warm_read <= READ_FROM_50_INMEM_BUDGET,
        "read_from(0, 50) from in-memory allocated {warm_read} events, \
         budget {READ_FROM_50_INMEM_BUDGET}"
    );

    // --- stats() ---
    let stats_allocs = count_allocs(|| {
        let _ = buf.stats();
    });
    assert!(
        stats_allocs <= STATS_BUDGET,
        "stats() allocated {stats_allocs} events, budget {STATS_BUDGET}"
    );

    // --- append that triggers a flush ---
    // Use a fresh buffer with Batch(1) so every append flushes.
    let tmp2 = TempDir::new().unwrap();
    let mut config2 = SegmentConfig::default();
    config2.flush_policy = FlushPolicy::Batch(1);
    config2.max_size_bytes = 100 * 1024 * 1024;
    config2.durability = DurabilityPolicy::Throughput;
    let buf2 = SegmentBuffer::open(tmp2.path(), config2).unwrap();
    let append_flush = count_allocs(|| {
        buf2.append(item(0)).unwrap();
    });
    assert!(
        append_flush <= APPEND_WITH_FLUSH_BUDGET,
        "append with flush allocated {append_flush} events, \
         budget {APPEND_WITH_FLUSH_BUDGET}"
    );

    eprintln!(
        "allocation counts — warm_append={warm_append} (budget {WARM_APPEND_BUDGET}), \
         read_from_50_inmem={warm_read} (budget {READ_FROM_50_INMEM_BUDGET}), \
         stats={stats_allocs} (budget {STATS_BUDGET}), \
         append_with_flush={append_flush} (budget {APPEND_WITH_FLUSH_BUDGET})"
    );
}
