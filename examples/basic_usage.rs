//! Basic segment-buffer usage: append items, flush, read back, delete acked.

use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Task {
    id: u64,
    title: String,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    let buffer = SegmentBuffer::<Task>::open(
        tmp.path(),
        SegmentConfig {
            max_batch_events: 256,
            flush_interval_secs: 5,
            max_size_bytes: 1024 * 1024,
            compression_level: 3,
            cipher: None,
        },
    )?;

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
