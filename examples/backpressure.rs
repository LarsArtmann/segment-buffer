//! Backpressure: use store_pressure() to implement a custom admission policy.
//!
//! This example demonstrates how a caller can apply its own priority-based
//! admission policy using the buffer's store_pressure() metric.

// SegmentConfig is #[non_exhaustive]: Default + field reassignment is the only
// external construction pattern; accept the clippy lint for that reason.
#![allow(clippy::field_reassign_with_default)]

use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Metric {
    name: String,
    value: f64,
}

/// Caller-defined priority levels (the crate ships no policy — it's yours to define).
enum Priority {
    Critical,
    Standard,
    Ephemeral,
}

/// A caller-defined admission policy based on store_pressure().
fn should_accept(priority: Priority, pressure: f32) -> bool {
    match priority {
        // Always accept critical data (security events, process info)
        Priority::Critical => true,
        // Reject standard data above 95% disk usage
        Priority::Standard => pressure < 0.95,
        // Reject ephemeral data above 90% disk usage (screenshots, camera)
        Priority::Ephemeral => pressure < 0.90,
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;

    // Small limit so we hit pressure quickly.
    let mut config = SegmentConfig::default();
    config.max_batch_events = 100_000; // don't auto-flush; we want to control it
    config.flush_interval_secs = 3600;
    config.max_size_bytes = 100_000; // 100 KB
    config.compression_level = 3;

    let buffer = SegmentBuffer::<Metric>::open(tmp.path(), config)?;

    let mut accepted = 0;
    let mut rejected = 0;

    for i in 0..10_000 {
        let priority = if i % 10 == 0 {
            Priority::Critical
        } else if i % 3 == 0 {
            Priority::Ephemeral
        } else {
            Priority::Standard
        };

        let pressure = buffer.store_pressure();
        if should_accept(priority, pressure) {
            buffer.append(Metric {
                name: format!("metric_{i}"),
                value: i as f64,
            })?;
            accepted += 1;
        } else {
            rejected += 1;
        }

        // Flush periodically so disk usage accrues
        if i % 500 == 0 {
            buffer.flush()?;
        }
    }
    buffer.flush()?;

    let final_pressure = buffer.store_pressure();
    println!("Accepted: {accepted}");
    println!("Rejected (backpressure): {rejected}");
    println!("Final disk pressure: {:.1}%", final_pressure * 100.0);
    println!("Overloaded: {}", buffer.is_overloaded());

    Ok(())
}
