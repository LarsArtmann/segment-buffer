//! Fuzz target: `FlushPolicy::should_flush` boolean logic over arbitrary
//! parameters must never panic, regardless of the policy variant, the
//! parameter values, the pending length, or the elapsed duration.
//!
//! Exercises edge cases like `min_batch > batch_size`, `max_interval <
//! interval`, zero/near-overflow sizes, and zero durations.
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_flush_policy
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::fuzz_hooks::{should_flush, FlushPolicy};
use std::time::Duration;

fuzz_target!(|data: &[u8]| {
    if data.len() < 17 {
        return;
    }
    // Byte 0: variant selector (0-4)
    // Bytes 1-8: pending_len (u64 LE)
    // Bytes 9-16: elapsed_nanos (u64 LE)
    // Remaining bytes: policy parameters
    let variant = data[0] % 5;
    let pending_len = u64::from_le_bytes(data[1..9].try_into().unwrap()) as usize;
    let elapsed_nanos = u64::from_le_bytes(data[9..17].try_into().unwrap());
    let elapsed = Duration::from_nanos(elapsed_nanos);

    let policy = match variant {
        0 => FlushPolicy::Batch(data.get(17).map_or(0, |b| usize::from(*b))),
        1 => FlushPolicy::Interval(elapsed),
        2 => {
            let batch = data.get(17).map_or(256, |b| usize::from(*b));
            let interval_nanos = data
                .get(18..26)
                .and_then(|s| s.try_into().ok())
                .map(u64::from_le_bytes)
                .unwrap_or(5_000_000_000);
            FlushPolicy::BatchOrInterval {
                batch_size: batch,
                interval: Duration::from_nanos(interval_nanos),
            }
        }
        3 => {
            let batch = data.get(17).map_or(256, |b| usize::from(*b));
            let min_batch = data.get(18).map_or(10, |b| usize::from(*b));
            let interval_nanos = data
                .get(19..27)
                .and_then(|s| s.try_into().ok())
                .map(u64::from_le_bytes)
                .unwrap_or(5_000_000_000);
            let max_interval_nanos = data
                .get(27..35)
                .and_then(|s| s.try_into().ok())
                .map(u64::from_le_bytes)
                .unwrap_or(60_000_000_000);
            FlushPolicy::BatchOrIntervalMin {
                batch_size: batch,
                min_batch,
                interval: Duration::from_nanos(interval_nanos),
                max_interval: Duration::from_nanos(max_interval_nanos),
            }
        }
        _ => FlushPolicy::Manual,
    };

    // Contract: should_flush must never panic on any input.
    let _ = should_flush(&policy, pending_len, elapsed);
});
