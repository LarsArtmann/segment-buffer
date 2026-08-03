//! Tuning `FlushPolicy::Batch(N)` using `segment_size_stats()`.
//!
//! `segment_size_stats()` is the observability primitive for batch-size
//! tuning — it reports the on-disk size distribution (min / p50 / p90 / max)
//! of segment files so you can answer: "is my batch size producing too many
//! tiny files (write amplification, inode pressure) or too few huge ones
//! (slow bounded reads, decode memory pressure)?"
//!
//! This demo walks through the full tuning loop:
//!
//! 1. **Problem** — start with a tiny batch, measure, observe the symptoms.
//! 2. **Sweep** — try a range of candidate batch sizes, classify each
//!    against a target segment-size window.
//! 3. **Recommendation** — pick the first batch size whose p50 lands inside
//!    the window.
//!
//! Run: `cargo run --example segment_tuning`

// Example/demo code: unwrap/expect, `as` conversions, and counter
// arithmetic are idiomatic in examples for clarity.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic_in_result_fn,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]

use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig, SegmentSizeStats};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::Path;

/// Segments smaller than this are "tiny" — too many files for the data they
/// hold, amplifying per-file write cost and inode usage. Tune to your
/// filesystem and workload.
const TARGET_MIN_BYTES: u64 = 4 * 1024;
/// Segments larger than this are "huge" — a single bounded read or decode
/// pulls in more data than necessary. Lower this if you read with small
/// `limit` values frequently.
const TARGET_MAX_BYTES: u64 = 256 * 1024;
/// Fixed workload so every candidate is measured against the same data.
const WORKLOAD: usize = 5_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Event {
    timestamp: u64,
    level: u8,
    source: String,
    message: String,
}

fn make_event(i: usize) -> Event {
    let services = ["api", "db", "cache", "queue"];
    Event {
        timestamp: 1_700_000_000 + i as u64,
        level: (i % 5) as u8,
        source: format!("host-{}/{}", i % 32, services[i % 4]),
        message: format!("processed request #{i:05} with payload data"),
    }
}

/// Open a fresh buffer in `dir`, write `WORKLOAD` events at the given batch
/// size, flush the tail, and return the on-disk segment size distribution.
fn measure(dir: &Path, batch_size: usize) -> Result<SegmentSizeStats, Box<dyn Error>> {
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Batch(batch_size))
        .compression_level(3)
        .build();
    let buf = SegmentBuffer::<Event>::open(dir, config)?;
    for i in 0..WORKLOAD {
        buf.append(make_event(i))?;
    }
    buf.flush()?;
    Ok(buf.segment_size_stats()?)
}

/// True when p50 sits inside the target window and no segment exceeds the max.
fn in_target_window(stats: &SegmentSizeStats) -> bool {
    stats.count > 0 && stats.p50_bytes >= TARGET_MIN_BYTES && stats.max_bytes <= TARGET_MAX_BYTES
}

/// One-word verdict for display.
fn assess(stats: &SegmentSizeStats) -> &'static str {
    if stats.count == 0 {
        "empty"
    } else if stats.p50_bytes < TARGET_MIN_BYTES {
        "too small"
    } else if stats.max_bytes > TARGET_MAX_BYTES {
        "too large"
    } else {
        "well-tuned"
    }
}

fn print_row(label: &str, batch: usize, stats: &SegmentSizeStats) {
    println!(
        "  {label:<10} Batch({batch:<5}) {:>4} segs  \
         p50 {:>6} B  p90 {:>6} B  max {:>6} B  -> {}",
        stats.count,
        stats.p50_bytes,
        stats.p90_bytes,
        stats.max_bytes,
        assess(stats),
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "Workload: {WORKLOAD} events | target segment size: \
         {TARGET_MIN_BYTES}-{TARGET_MAX_BYTES} B\n"
    );

    // -- Phase 1: the problem --
    // A naive starting point. With only 8 items per flush the crate writes
    // hundreds of tiny segment files — exactly the symptom segment_size_stats
    // is designed to surface.
    let dir = tempfile::tempdir()?;
    let stats = measure(dir.path(), 8)?;
    println!("Phase 1 - initial guess (tiny batch):");
    print_row("initial", 8, &stats);

    // -- Phase 2: sweep candidate batch sizes --
    // For each candidate we write the *same* workload into a fresh directory
    // so the measurement is not contaminated by left-over segments. The first
    // candidate whose p50 lands inside the target window wins.
    println!("\nPhase 2 - sweep candidate batch sizes:");
    let candidates = [32, 64, 128, 256, 512, 1024];
    let mut winner: Option<(usize, SegmentSizeStats)> = None;
    for &batch in &candidates {
        let dir = tempfile::tempdir()?;
        let stats = measure(dir.path(), batch)?;
        print_row("candidate", batch, &stats);
        if in_target_window(&stats) && winner.is_none() {
            winner = Some((batch, stats));
        }
    }

    // -- Phase 3: recommendation --
    println!("\nPhase 3 - recommendation:");
    match winner {
        Some((batch, stats)) => {
            println!("  FlushPolicy::Batch({batch})");
            println!(
                "  {} segments, p50 {} B, max {} B - inside target window.",
                stats.count, stats.p50_bytes, stats.max_bytes,
            );
        }
        None => {
            println!(
                "  No candidate landed p50 inside [{TARGET_MIN_BYTES}, \
                 {TARGET_MAX_BYTES}] B."
            );
            println!(
                "  Widen the target window or try batch sizes outside the \
                 swept range."
            );
        }
    }

    Ok(())
}
