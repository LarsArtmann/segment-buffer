//! Crash-recovery demo: shows that segment files survive a "crash" (a process
//! restart = dropping the SegmentBuffer and re-opening), and that the recovery
//! scan rebuilds head_seq / next_seq from filenames without reading segment
//! contents.
//!
//! Run with: `cargo run --example crash_recovery`

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use tempfile::tempdir;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Event {
    seq: u64,
    payload: String,
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    println!("Using dir: {}", dir.path().display());

    // ---- Phase 1: write some events and flush them so they hit disk ----
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual) // we control durability explicitly
        .compression_level(3)
        .build();
    let buf: SegmentBuffer<Event> = SegmentBuffer::open(dir.path(), config)?;

    for i in 0..5u64 {
        buf.append(Event {
            seq: i,
            payload: format!("event-{i}"),
        })?;
    }
    // Manual flush = fsync + atomic rename. After this, the segment exists on
    // disk even if the process is killed with SIGKILL.
    buf.flush()?;
    println!("[phase 1] wrote 5 events and flushed them to disk");

    // List what's on disk after the flush.
    for entry in std::fs::read_dir(dir.path())? {
        let entry = entry?;
        println!(
            "  on-disk after flush: {} ({} bytes)",
            entry.file_name().to_string_lossy(),
            entry.metadata()?.len()
        );
    }

    // ---- Phase 2: append more events WITHOUT flushing ("crash" window) ----
    for i in 5..10u64 {
        buf.append(Event {
            seq: i,
            payload: format!("event-{i}"),
        })?;
    }
    println!("[phase 2] appended 5 more events (NOT flushed — will be lost on crash)");
    println!(
        "  in-memory pending_count before crash: {}",
        buf.pending_count()
    );

    // Drop the buffer without flushing — this simulates a crash. In a real
    // crash (SIGKILL, power loss), the OS-level filesystem journal guarantees
    // the same outcome: flushed segments survive, unflushed don't.
    drop(buf);
    println!("[phase 3] dropped the SegmentBuffer (simulated crash)");

    // ---- Phase 4: re-open and observe what recovery found ----
    let (buf2, report) =
        SegmentBuffer::<Event>::open_with_report(dir.path(), SegmentConfig::default())?;
    println!("[phase 4] re-opened; RecoveryReport:");
    println!("    segment_count  = {}", report.segment_count);
    println!("    head_seq       = {}", report.head_seq);
    println!("    next_seq       = {}", report.next_seq);
    println!("    disk_bytes     = {}", report.disk_bytes);
    println!("    removed_tmp    = {}", report.removed_tmp_files);

    // ---- Phase 5: read everything that survived ----
    let recovered = buf2.read_from(0, 1000)?;
    println!(
        "[phase 5] recovered {} events; their sequences:",
        recovered.len()
    );
    for ev in &recovered {
        println!("    seq={} payload={:?}", ev.seq, ev.payload);
    }
    assert_eq!(recovered.len(), 5, "only the flushed events should survive");
    assert_eq!(recovered[0].seq, 0);
    assert_eq!(recovered[4].seq, 4);

    println!();
    println!("TAKEAWAY: the 5 flushed events survived; the 5 unflushed did not.");
    println!("Recovery cost = one directory scan + one stat per segment, no segment reads.");

    Ok(())
}
