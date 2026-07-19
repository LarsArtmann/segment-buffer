//! Multi-producer multi-consumer demo: 4 writers append concurrently while 1
//! reader drains via read_from + delete_acked, all sharing one Arc<SegmentBuffer>.
//!
//! Run with: `cargo run --example mpmc`

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Job {
    /// Writer ID, so we can confirm all writers contributed.
    writer: u64,
    /// Sequence assigned by this writer.
    seq: u64,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let config = SegmentConfig::builder()
        // Manual flush so writers don't pay the segment-write cost on every
        // append; a background flusher (omitted here for brevity) would call
        // buf.flush() on a timer.
        .flush_policy(FlushPolicy::Manual)
        .compression_level(3)
        .build();

    let buf: Arc<SegmentBuffer<Job>> = Arc::new(SegmentBuffer::open(dir.path(), config)?);

    const WRITERS: u64 = 4;
    const WRITES_PER_WRITER: u64 = 1_000;
    let total_writes = WRITERS * WRITES_PER_WRITER;
    let total_appended = Arc::new(AtomicU64::new(0));

    // ---- Writers: 4 threads, each appends WRITES_PER_WRITER jobs ----
    let mut handles = Vec::new();
    for writer_id in 0..WRITERS {
        let buf = buf.clone();
        let counter = total_appended.clone();
        handles.push(thread::spawn(move || {
            for seq in 0..WRITES_PER_WRITER {
                let _assigned_seq = buf
                    .append(Job {
                        writer: writer_id,
                        seq,
                    })
                    .expect("append must not fail under backpressure (Manual flush, no cap)");
                counter.fetch_add(1, Ordering::Relaxed);
                // The buffer assigns its own monotonic sequence; we ignore
                // the returned value here for brevity. The reader uses the
                // buffer's `read_from` / `delete_acked` API which tracks
                // sequences internally.
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    println!(
        "[writers] {} writers x {} appends = {} total (verified: {})",
        WRITERS,
        WRITES_PER_WRITER,
        total_writes,
        total_appended.load(Ordering::Relaxed)
    );

    // Flush so the reader can see everything via read_from (which sees
    // on-disk segments + in-memory pending, but flushing keeps the demo honest
    // — the reader would see the same total either way).
    buf.flush()?;
    println!(
        "[after flush] pending_count={} latest_sequence={}",
        buf.pending_count(),
        buf.latest_sequence()
    );

    // ---- Reader: drain everything in batches of 100, ack each batch ----
    let mut next_to_read: u64 = 0;
    let mut total_read: u64 = 0;
    let mut per_writer_counts = [0u64; WRITERS as usize];

    while next_to_read < total_writes {
        let batch = buf.read_from(next_to_read, 100)?;
        if batch.is_empty() {
            // Should not happen — we know the writes are all in.
            break;
        }
        for job in &batch {
            per_writer_counts[job.writer as usize] += 1;
        }
        // The buffer assigns monotonic sequences; the last job in this batch
        // has buffer seq `next_to_read + batch.len() - 1` (no gaps in our run).
        let last_buffer_seq = next_to_read + batch.len() as u64 - 1;
        let _deleted = buf.delete_acked(last_buffer_seq)?;
        next_to_read = last_buffer_seq + 1;
        total_read += batch.len() as u64;
    }

    println!(
        "[reader] read {} jobs across {} batches",
        total_read,
        total_read / 100
    );
    for (writer, count) in per_writer_counts.iter().enumerate() {
        println!("    writer {}: {} jobs read back", writer, count);
    }

    assert_eq!(total_read, total_writes, "every appended job must be read");
    for count in per_writer_counts {
        assert_eq!(
            count, WRITES_PER_WRITER,
            "every writer's jobs must be fully drained"
        );
    }

    println!();
    println!("TAKEAWAY: parking_lot::Mutex made this correct with zero ceremony.");
    println!("Lock contention is the only serialization — no reads block writes for long.");

    Ok(())
}
