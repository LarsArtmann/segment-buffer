//! Basic segment-buffer usage: append items, flush, read back, delete acked.

// SegmentConfig is #[non_exhaustive]: Default + field reassignment is the only
// external construction pattern; accept the clippy lint for that reason.
#![allow(clippy::field_reassign_with_default)]

use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Task {
    id: u64,
    title: String,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    let mut config = SegmentConfig::default();
    config.max_batch_events = 256;
    config.flush_interval_secs = 5;
    config.max_size_bytes = 1024 * 1024;
    config.compression_level = 3;

    let buffer = SegmentBuffer::<Task>::open(tmp.path(), config)?;

    // Append some tasks
    for i in 0..10 {
        buffer.append(Task {
            id: i,
            title: format!("Task {i}"),
        })?;
    }

    // Explicit flush (otherwise auto-flushes at batch threshold or interval)
    buffer.flush()?;

    println!("Pending: {}", buffer.pending_count());
    println!("Latest sequence: {}", buffer.latest_sequence());

    // Read all tasks back
    let tasks = buffer.read_from(0, 100)?;
    println!("Recovered {} tasks", tasks.len());
    assert_eq!(tasks.len(), 10);
    assert_eq!(
        tasks[0],
        Task {
            id: 0,
            title: "Task 0".into()
        }
    );

    // Acknowledge all tasks — the segment is deleted when end_seq <= acked_seq
    let deleted = buffer.delete_acked(9)?;
    println!("Deleted {deleted} segment(s)");
    println!("Pending after ack: {}", buffer.pending_count());

    Ok(())
}
