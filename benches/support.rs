//! Shared helpers for the criterion benchmark targets.
//!
//! Each benchmark file is compiled as a separate binary, so we pull this
//! module in via `#[path = "support.rs"] mod support;` — it is never built
//! on its own.
//!
//! These helpers are bench-internal and not part of the crate's public API;
//! `missing_panics_doc` / `missing_errors_doc` are not enforced here.

// Non-production code: unwrap/expect on setup failures, `as` conversions for
// batch sizes, and counter arithmetic are idiomatic and safe here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

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
#[must_use]
pub fn item(n: u64) -> Item {
    Item {
        id: n,
        payload: format!("payload-{n}"),
    }
}

/// Build a text-like [`Item`] with realistic English-prose-shaped payload.
#[allow(dead_code)] // only bench_append_realistic uses this
#[must_use]
pub fn text_item(n: u64) -> Item {
    Item {
        id: n,
        payload: format!(
            "Event {n}: The system collected metrics at the configured interval. \
             CPU usage was nominal, memory remained within bounds, and all \
             subsystems reported healthy status. Timestamp: 2026-08-11T{n:06}."
        ),
    }
}

/// Build a JSON-like [`Item`] with structured payload.
#[allow(dead_code)] // only bench_append_realistic uses this
#[must_use]
pub fn json_item(n: u64) -> Item {
    Item {
        id: n,
        payload: format!(
            r#"{{"id":{n},"ts":"2026-08-11T12:00:{n:06}","level":"info","module":"collector","msg":"metric tick","cpu":42.5,"mem":1073741824,"disk":8589934592,"net":{{"rx":1234567,"tx":9876543}}}}"#
        ),
    }
}

/// The shared benchmark config. The `flush_at_batch` argument is the only knob
/// that varies between benchmarks, so it is the single parameter; everything
/// else is pinned for cross-target consistency.
#[must_use]
pub fn config(flush_at_batch: usize) -> SegmentConfig {
    SegmentConfig::builder()
        .flush_at_batch_size(flush_at_batch)
        .max_size_bytes(u64::MAX)
        .compression_level(1)
        .build()
}

/// Open a buffer in a fresh temp directory using [`config`].
#[must_use]
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
#[must_use]
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
