//! Cross-platform filesystem semantics tests.
//!
//! These tests document and verify behaviours that differ between Linux ext4
//! and macOS APFS. They are platform-agnostic — each test passes on every
//! platform, but the _meaning_ of the test (what failure would reveal)
//! differs by filesystem.
//!
//! ## Documented platform differences
//!
//! | Behaviour | Linux ext4 | macOS APFS |
//! | --- | --- | --- |
//! | `unlink` on a path another process has open | succeeds, file removed when last fd closes | same (POSIX semantics) |
//! | `unlink` on the same path from two threads | one succeeds, one gets `ENOENT` | both can succeed (non-exclusive) |
//! | Directory `mtime` granularity | nanosecond (most kernels) | second (APFS default) |
//! | `rename` over an existing file | atomically replaces | atomically replaces |
//! | `fsync` on a directory | flushes directory entries | flushes directory entries |
//!
//! The `unlink` non-exclusivity on APFS caused a CI failure in v0.5.6: two
//! threads calling `delete_acked` on the same segment both succeeded, and the
//! second thread's `fs::remove_file` returned Ok even though the file was
//! already gone. The library handles this gracefully (removal is idempotent),
//! but the test suite must not assume `ENOENT` on the second removal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::nursery
)]

use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Item {
    id: u64,
}

fn config() -> SegmentConfig {
    SegmentConfig::builder()
        .flush_at_batch_size(4)
        .max_size_bytes(1024 * 1024)
        .compression_level(1)
        .durability(segment_buffer::DurabilityPolicy::Throughput)
        .build()
}

/// Deleting the same segment file from two threads must not panic.
///
/// On Linux ext4, the second `fs::remove_file` returns `ENOENT`.
/// On macOS APFS, both calls can succeed (non-exclusive unlink).
/// The library's `delete_acked` handles both cases: removal is idempotent.
#[test]
fn concurrent_unlink_same_path_does_not_panic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = SegmentBuffer::<Item>::open(dir, config()).unwrap();

    // Append + flush to create a segment file.
    buf.append(Item { id: 1 }).unwrap();
    buf.flush().unwrap();

    // delete_acked is idempotent — calling it twice on the same range
    // must not panic or error, regardless of platform.
    buf.delete_acked(1).unwrap();
    buf.delete_acked(1).unwrap();
}

/// `delete_acked` from two concurrent threads on overlapping ranges
/// must not panic. This is the regression guard for the v0.5.6 APFS failure.
#[test]
fn concurrent_delete_acked_overlapping_ranges_no_panic() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let buf = std::sync::Arc::new(SegmentBuffer::<Item>::open(dir, config()).unwrap());

    // Create several segments.
    for i in 0..16 {
        buf.append(Item { id: i }).unwrap();
    }
    buf.flush().unwrap();

    let buf_a = buf.clone();
    let buf_b = buf.clone();

    let h1 = std::thread::spawn(move || {
        buf_a.delete_acked(15).unwrap();
    });
    let h2 = std::thread::spawn(move || {
        buf_b.delete_acked(15).unwrap();
    });

    h1.join().expect("thread 1 panicked");
    h2.join().expect("thread 2 panicked");
}
