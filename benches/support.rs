//! Shared helpers for the criterion benchmark targets.
//!
//! Each benchmark file is compiled as a separate binary, so we pull this
//! module in via `#[path = "support.rs"] mod support;` — it is never built
//! on its own.
//!
//! These helpers are bench-internal and not part of the crate's public API;
//! `missing_panics_doc` / `missing_errors_doc` are not enforced here.

use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

/// Canonical benchmark item: a small serializable record.
#[derive(Serialize, Deserialize, Clone)]
pub struct Item {
    /// Sequence-equivalent identifier.
    pub id: u64,
    /// Variable-length payload, mirrors real-world record shape.
    pub payload: String,
}

/// Build [`Item`] number `n` with a recognizable payload.
pub fn item(n: u64) -> Item {
    Item {
        id: n,
        payload: format!("payload-{n}"),
    }
}

/// The shared benchmark config. The `flush_at_batch` argument is the only knob
/// that varies between benchmarks, so it is the single parameter; everything
/// else is pinned for cross-target consistency.
pub fn config(flush_at_batch: usize) -> SegmentConfig {
    SegmentConfig::builder()
        .flush_at_batch_size(flush_at_batch)
        .max_size_bytes(u64::MAX)
        .compression_level(3)
        .build()
}

/// Open a buffer in a fresh temp directory using [`config`].
pub fn open_buffer(flush_at_batch: usize) -> (SegmentBuffer<Item>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let buf = SegmentBuffer::<Item>::open(tmp.path(), config(flush_at_batch)).unwrap();
    (buf, tmp)
}

/// Open a buffer and pre-populate it with `n_segments` segment files on disk,
/// each holding `items_per_segment` items.
///
/// Used by `bench_read_from` to measure the `scan_segments` cache against a
/// realistic directory size. The flush policy is set to `Batch(items_per_segment)`
/// so each batch lands as its own segment file; the explicit `flush()` after
/// every batch is belt-and-braces for the partial tail.
#[allow(dead_code)] // only bench_read_from uses this; other bench binaries see it as dead
pub fn open_buffer_with_segments(
    n_segments: usize,
    items_per_segment: usize,
) -> (SegmentBuffer<Item>, tempfile::TempDir) {
    let (buf, tmp) = open_buffer(items_per_segment);
    for s in 0..n_segments {
        let base = (s * items_per_segment) as u64;
        for i in 0..items_per_segment as u64 {
            buf.append(item(base + i)).unwrap();
        }
        buf.flush().unwrap();
    }
    (buf, tmp)
}
