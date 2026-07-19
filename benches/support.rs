//! Shared helpers for the criterion benchmark targets.
//!
//! Each benchmark file is compiled as a separate binary, so we pull this
//! module in via `#[path = "support.rs"] mod support;` — it is never built
//! on its own.

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

/// The shared benchmark config. `max_batch_events` is the only knob that varies
/// between benchmarks, so it is the single parameter; everything else is pinned
/// for cross-target consistency.
pub fn config(max_batch_events: usize) -> SegmentConfig {
    SegmentConfig {
        max_batch_events,
        flush_interval_secs: 3600,
        max_size_bytes: u64::MAX,
        compression_level: 3,
        cipher: None,
    }
}

/// Open a buffer in a fresh temp directory using [`config`].
pub fn open_buffer(max_batch_events: usize) -> (SegmentBuffer<Item>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let buf = SegmentBuffer::<Item>::open(tmp.path(), config(max_batch_events)).unwrap();
    (buf, tmp)
}
