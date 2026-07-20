//! Cloud-sync drain loop: the at-least-once delivery pattern this crate is built for.
//!
//! This example demonstrates the canonical producer-side pattern:
//!
//! 1. Open a [`SegmentBuffer`] with `DurabilityPolicy::Throughput` (the cloud
//!    is the durable layer; the local disk is a throughput buffer).
//! 2. Produce items with `append` + `flush`.
//! 3. Drain with a `read_from(cursor, N) → upload → delete_acked` loop.
//! 4. Survive transient upload failures by retrying the same batch (at-least-once).
//!
//! Run with: `cargo run --example cloud_sync`

use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Event {
    id: u64,
    payload: String,
}

/// The cloud endpoint the drain loop uploads to.
///
/// In production this would be an HTTP client (reqwest, hyper, aws-sdk-s3,
/// etc.) with retry policy, auth, batching, and idempotency keys. Here we
/// fake it with an in-memory sink so the example is runnable offline.
trait CloudUploader: Send + Sync {
    /// Upload a batch of events starting at `start_seq`. Returns the number
    /// of bytes the cloud endpoint acknowledged, or `Err` if the upload
    /// failed and the batch should be retried.
    fn upload(&self, start_seq: u64, batch: &[Event]) -> Result<usize, String>;
}

/// Successful uploader: every call succeeds. Used to demonstrate the happy
/// path of the drain loop.
struct ReliableUploader {
    received: AtomicU64,
}

impl CloudUploader for ReliableUploader {
    fn upload(&self, _start_seq: u64, batch: &[Event]) -> Result<usize, String> {
        let bytes: usize = batch.iter().map(|e| e.payload.len()).sum();
        self.received
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
        Ok(bytes)
    }
}

/// Flaky uploader: fails the first two calls per `start_seq`, then succeeds.
/// Models a transient cloud outage (network blip, rate-limit, brief 5xx
/// window) that the drain loop must retry through WITHOUT skipping any
/// events. This is the at-least-once contract in action.
struct FlakyUploader {
    received: AtomicU64,
    failed_once_for: Mutex<std::collections::HashSet<u64>>,
}

impl CloudUploader for FlakyUploader {
    fn upload(&self, start_seq: u64, batch: &[Event]) -> Result<usize, String> {
        let mut seen = self.failed_once_for.lock().unwrap();
        if !seen.contains(&start_seq) {
            // First attempt for this batch — simulate transient failure.
            seen.insert(start_seq);
            return Err(format!(
                "transient cloud outage on batch starting at seq {start_seq}"
            ));
        }
        let bytes: usize = batch.iter().map(|e| e.payload.len()).sum();
        self.received
            .fetch_add(batch.len() as u64, Ordering::Relaxed);
        Ok(bytes)
    }
}

/// The drain loop. Reads a batch, uploads it, and on success advances the
/// cursor and acknowledges the segment(s) so they can be deleted. On
/// failure, retries the same batch — `read_from(cursor, N)` will return
/// the same items because we have not called `delete_acked` yet.
///
/// `max_retries` bounds the loop so a permanently-failing cloud endpoint
/// does not spin forever. In production this is your backoff-and-give-up
/// threshold.
fn drain_loop(
    buf: &SegmentBuffer<Event>,
    uploader: &dyn CloudUploader,
    cursor: &mut u64,
    max_retries: u32,
) -> Result<(u64, u32), String> {
    let batch_size = 500usize;
    let mut uploaded = 0u64;
    let mut retries = 0u32;
    loop {
        let batch = buf
            .read_from(*cursor, batch_size)
            .map_err(|e| format!("read_from failed: {e}"))?;
        if batch.is_empty() {
            break; // drained
        }
        let last_seq = *cursor + batch.len() as u64 - 1;

        let mut succeeded = false;
        for attempt in 0..=max_retries {
            match uploader.upload(*cursor, &batch) {
                Ok(_bytes) => {
                    succeeded = true;
                    uploaded += batch.len() as u64;
                    break;
                }
                Err(e) => {
                    retries += 1;
                    if attempt == max_retries {
                        return Err(format!(
                            "upload failed after {max_retries} retries at seq {}: {e}",
                            *cursor
                        ));
                    }
                    eprintln!("  retry {attempt} at seq {}: {e}", *cursor);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
        if !succeeded {
            unreachable!("either succeeded or returned Err in the loop above");
        }

        // Acknowledge the batch — delete_acked removes every segment whose
        // end <= last_seq. This is the commit point: items are now durably
        // in the cloud and safe to remove locally.
        let _ = buf
            .delete_acked(last_seq)
            .map_err(|e| format!("delete_acked failed: {e}"))?;
        *cursor = last_seq + 1;
    }
    Ok((uploaded, retries))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    // Throughput policy: skip per-flush fsync because the cloud is the
    // durable layer. This is the canonical cloud-sync deployment choice.
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Batch(500))
        .durability(DurabilityPolicy::Throughput)
        .build();
    let buf: SegmentBuffer<Event> = SegmentBuffer::open(tmp.path(), config)?;

    // Produce 10_000 events.
    const TOTAL: u64 = 10_000;
    for id in 0..TOTAL {
        buf.append(Event {
            id,
            payload: format!("event-{id}"),
        })?;
    }
    buf.flush()?;
    println!(
        "produced {TOTAL} events; pending_count = {}",
        buf.pending_count()
    );

    // Drain with a reliable uploader: zero retries expected.
    let mut cursor = buf.stats().head_sequence;
    println!("\n--- drain with ReliableUploader ---");
    let (uploaded, retries) = drain_loop(
        &buf,
        &ReliableUploader {
            received: AtomicU64::new(0),
        },
        &mut cursor,
        3,
    )?;
    println!(
        "uploaded {uploaded} events, {retries} retries; pending_count = {}",
        buf.pending_count()
    );
    assert_eq!(uploaded, TOTAL);
    assert_eq!(retries, 0);

    // Re-populate and drain with the flaky uploader: each batch fails once
    // then succeeds. This demonstrates that the at-least-once loop survives
    // transient failures without skipping events.
    for id in 0..TOTAL {
        buf.append(Event {
            id: id + TOTAL, // offset so the ids are distinct from the first run
            payload: format!("event2-{id}"),
        })?;
    }
    buf.flush()?;
    let mut cursor = buf.stats().head_sequence;
    println!("\n--- drain with FlakyUploader (transient failures) ---");
    let flaky = FlakyUploader {
        received: AtomicU64::new(0),
        failed_once_for: Mutex::new(std::collections::HashSet::new()),
    };
    let (uploaded, retries) = drain_loop(&buf, &flaky, &mut cursor, 3)?;
    let received = flaky.received.load(Ordering::Relaxed);
    println!(
        "uploaded {uploaded} events (cloud-side: {received} received), {retries} retries; \
         pending_count = {}",
        buf.pending_count()
    );
    // At-least-once: the cloud endpoint saw every event at least once.
    // Idempotency on (producer, seq) is the SERVER's concern, not the
    // buffer's. See examples/idempotent_server.rs for that pattern.
    assert_eq!(uploaded, TOTAL);
    assert_eq!(received, TOTAL);
    assert!(
        retries >= 20,
        "flaky uploader should have forced at least one retry per batch"
    );

    println!("\nAll drained. The buffer directory now contains only the lock sidecar.");
    Ok(())
}
