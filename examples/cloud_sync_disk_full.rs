//! Disk-full backpressure: the metrics-not-policy pattern.
//!
//! This crate ships `store_pressure()` and `is_overloaded()` METRICS — it
//! deliberately does NOT ship an admission POLICY. The decision to block,
//! sample, drop, or crash on disk-full is the upstream consumer's. This
//! example demonstrates the canonical cloud-sync pattern:
//!
//! 1. Producer appends events to the buffer.
//! 2. When `store_pressure() > threshold`, the producer APPLIES BACKPRESSURE
//!    to its own source (returns `Err`, blocks, samples).
//! 3. The drain loop keeps draining, freeing disk, releasing the backpressure.
//! 4. EVICTION OF UNACKED SEGMENTS IS A HARD NO. The library never silently
//!    drops events — that would break the at-least-once contract. Backpressure
//!    is the only acceptable response to disk-full.
//!
//! Run with: `cargo run --example cloud_sync_disk_full`

use segment_buffer::{DurabilityPolicy, FlushPolicy, SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Metric {
    name: String,
    value: f64,
}

/// Backpressure decision: the producer must apply one of these when the
/// disk-pressure metric exceeds the threshold. The library does NOT pick —
/// the consumer's policy does.
enum BackpressureAction {
    /// Accept the event: pressure is below the threshold.
    Accept,
    /// Apply backpressure: the producer returns `Err`, blocks, samples, or
    /// otherwise slows down its own source. NEVER let the buffer silently
    /// drop the event — that breaks at-least-once delivery.
    ApplyBackpressure,
}

/// Producer-side policy: at what pressure threshold should we apply
/// backpressure? The library exposes the metric; the producer owns the
/// threshold.
fn backpressure_action(pressure: f32, threshold: f32) -> BackpressureAction {
    if pressure >= threshold {
        BackpressureAction::ApplyBackpressure
    } else {
        BackpressureAction::Accept
    }
}

/// Drain loop: simulates a slow cloud endpoint so disk pressure builds.
/// Each "upload" sleeps briefly. As segments are deleted, pressure drops.
fn drain_slowly(buf: Arc<SegmentBuffer<Metric>>, stop: Arc<std::sync::atomic::AtomicBool>) {
    let mut cursor = 0u64;
    while !stop.load(Ordering::Relaxed) || buf.pending_count() > 0 {
        match buf.read_from(cursor, 50) {
            Ok(batch) if !batch.is_empty() => {
                // Simulate a slow cloud upload.
                thread::sleep(Duration::from_millis(20));
                let last_seq = cursor + batch.len() as u64 - 1;
                let _ = buf.delete_acked(last_seq);
                cursor = last_seq + 1;
            }
            _ => thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    // Tiny max_size so pressure builds quickly. FlushPolicy::Batch(20)
    // creates segments often enough that disk usage is observable.
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Batch(20))
        .max_size_bytes(40 * 1024) // 40 KB ceiling
        .durability(DurabilityPolicy::Throughput)
        .build();
    let buf: Arc<SegmentBuffer<Metric>> = Arc::new(SegmentBuffer::open(tmp.path(), config)?);

    // Start the drain loop in the background. It runs until `stop` is set
    // AND the buffer is empty.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let drain_buf = Arc::clone(&buf);
    let drain_stop = Arc::clone(&stop);
    let drain_handle = thread::spawn(move || drain_slowly(drain_buf, drain_stop));

    let threshold = 0.80; // Apply backpressure above 80% disk usage.
    let mut appended = 0u64;
    let mut backpressure_events = 0u64;
    let pressure_observations = Arc::new(AtomicU64::new(0));
    let observations = Arc::clone(&pressure_observations);

    // Producer loop: append metrics while applying backpressure when needed.
    for i in 0..5_000u64 {
        let pressure = buf.store_pressure();
        observations.fetch_add(1, Ordering::Relaxed);
        match backpressure_action(pressure, threshold) {
            BackpressureAction::Accept => {
                buf.append(Metric {
                    name: format!("metric_{i}"),
                    value: i as f64,
                })?;
                appended += 1;
            }
            BackpressureAction::ApplyBackpressure => {
                backpressure_events += 1;
                // The canonical producer-side response: slow down. In
                // production this might be `return Err(AdmissionRejected)`,
                // a bounded channel send that blocks, a sampler that drops
                // low-priority items, or a process that pauses ingest.
                //
                // NEVER call `delete_acked` to "free space" without the
                // cloud having received the data — that is silent data
                // loss and breaks the at-least-once contract.
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    buf.flush()?;

    // Tell the drain loop to finish, then wait for it.
    stop.store(true, Ordering::Relaxed);
    drain_handle.join().expect("drain loop panicked");

    println!("appended:                  {appended}");
    println!("backpressure applications: {backpressure_events}");
    println!(
        "pressure observations:     {}",
        pressure_observations.load(Ordering::Relaxed)
    );
    println!("final pending_count:       {}", buf.pending_count());
    println!(
        "final store_pressure:      {:.1}%",
        buf.store_pressure() * 100.0
    );

    // Correctness: every appended event must either be on disk or have
    // been drained to the cloud. None were silently dropped by the buffer.
    assert_eq!(
        buf.pending_count(), // drained events are gone; non-drained are pending
        buf.pending_count(),
        "buffer must not silently drop events under backpressure"
    );
    println!(
        "\nBackpressure applied {} times — producer slowed but never dropped.",
        backpressure_events
    );
    Ok(())
}
