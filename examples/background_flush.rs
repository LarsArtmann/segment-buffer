//! Background flush: decouple append latency from the flush cost using
//! `FlushPolicy::Manual` + a caller-owned timer thread.
//!
//! The default `FlushPolicy::Batch(N)` flushes inline on the
//! threshold-crossing `append()` call. That caller pays the full
//! CBOR → zstd → write cost. For p99-sensitive producers, decouple
//! the flush by setting `FlushPolicy::Manual` and flushing from a
//! dedicated thread on your own schedule.
//!
//! This is the **recommended pattern for latency-sensitive producers**.
//! It achieves the same decoupling as a library-internal worker would,
//! without adding a per-buffer thread, a channel, or delayed error
//! propagation to the crate. You own the thread, the schedule, and the
//! error handling.

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Event {
    id: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    let config = SegmentConfig::builder()
        // Manual: append() never auto-flushes. The timer thread below
        // owns the flush schedule. This keeps every append() at
        // "lock + Vec push + unlock" cost — no encode, no I/O.
        .flush_policy(FlushPolicy::Manual)
        .max_size_bytes(1024 * 1024)
        .build();

    let buffer = Arc::new(SegmentBuffer::<Event>::open(tmp.path(), config)?);

    // --- Background flusher thread (caller-owned) ---
    // The shutdown flag is how the main thread tells the flusher to stop;
    // without it, join() would wait forever on the infinite loop.
    let shutdown = Arc::new(AtomicBool::new(false));
    let flusher_buf = Arc::clone(&buffer);
    let flusher_shutdown = Arc::clone(&shutdown);
    let flusher = thread::Builder::new()
        .name("segment-buffer-flusher".into())
        .spawn(move || {
            // Flush every 50ms. flush() is a no-op when nothing is
            // buffered, so this is cheap on an empty buffer.
            while !flusher_shutdown.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(50));
                if let Err(e) = flusher_buf.flush() {
                    eprintln!("background flush failed: {e}");
                    break;
                }
            }
        })?;

    // --- Producer: append as fast as possible ---
    // Every append() is O(1) — no flush cost on the hot path.
    for i in 0..10_000 {
        buffer.append(Event { id: i })?;
    }

    // --- Shutdown: signal the flusher to stop, then do a final
    // synchronous flush to guarantee everything is on disk before exit.
    shutdown.store(true, Ordering::Relaxed);
    flusher.join().expect("flusher panicked");
    buffer.flush()?;

    // Verify all 10_000 items are durable.
    let items = buffer.read_from(0, 20_000)?;
    println!("Recovered {} items", items.len());
    assert_eq!(items.len(), 10_000);

    Ok(())
}
