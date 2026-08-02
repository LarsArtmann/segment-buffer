//! Demonstrates `FlushPolicy::BatchOrIntervalMin` — the policy that prevents
//! tiny segment files during low-throughput periods.
//!
//! Unlike `BatchOrInterval` (which flushes every `interval` regardless of how
//! few items are pending), `BatchOrIntervalMin` only flushes at the interval
//! if at least `min_batch` items have accumulated. A `max_interval` safety
//! valve ensures items don't sit in memory indefinitely.
//!
//! Run: `cargo run --example batch_or_interval_min`

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Event {
    seq: u64,
    payload: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    // BatchOrIntervalMin config:
    //   batch_size=100   — flush immediately when 100 items accumulate
    //   min_batch=10     — interval flushes need at least 10 items
    //   interval=5s      — check every 5 seconds
    //   max_interval=60s — safety valve: flush everything after 60 seconds
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::BatchOrIntervalMin {
            batch_size: 100,
            min_batch: 10,
            interval: Duration::from_secs(5),
            max_interval: Duration::from_secs(60),
        })
        .max_size_bytes(10 * 1024 * 1024)
        .compression_level(3)
        .build();

    let buf: SegmentBuffer<Event> = SegmentBuffer::open(tmp.path(), config)?;

    // Phase 1: Burst — items accumulate fast and immediately hit batch_size
    // (100), triggering an instant auto-flush to a single segment file.
    for i in 0..100 {
        buf.append(Event {
            seq: i,
            payload: format!("burst-event-{i}"),
        })?;
    }
    println!(
        "After 100-item burst: {} segment file on disk",
        count_segments(tmp.path())?
    );

    // Phase 2: Drip — append a few items (below min_batch). With the default
    // BatchOrInterval policy, the next 5-second interval would create a tiny
    // 3-item segment file. With BatchOrIntervalMin, these items stay in
    // memory because they haven't reached min_batch (10). No new segment file
    // is created until:
    //   - 7 more items arrive AND interval elapses (total >= min_batch), or
    //   - batch_size items accumulate (instant flush), or
    //   - max_interval (60s) elapses (safety valve flushes everything).
    for i in 100..103 {
        buf.append(Event {
            seq: i,
            payload: format!("drip-{i}"),
        })?;
    }
    println!(
        "After 3-item drip (below min_batch): {} segment file on disk",
        count_segments(tmp.path())?
    );
    println!("Total backlog (pending_count): {}", buf.pending_count());

    // For the demo, flush the remaining items manually.
    buf.flush()?;
    println!(
        "After manual flush: {} segment files on disk",
        count_segments(tmp.path())?
    );

    // Read everything back to verify data integrity.
    let all = buf.read_from(0, usize::MAX)?;
    println!("Total items readable: {}", all.len());

    Ok(())
}

fn count_segments(dir: &std::path::Path) -> Result<usize, Box<dyn std::error::Error>> {
    let count = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("seg_") && name.ends_with(".zst")
        })
        .count();
    Ok(count)
}
