//! Fuzz target: `for_each_from` must never panic and must never visit
//! out-of-order or seq-mismatched items, regardless of the start sequence,
//! limit, buffer state (empty, partially flushed, fully flushed, post-delete),
//! or interleaved mutations (flush / delete_acked / append) between calls.
//!
//! This complements `fuzz_append_all` (which fuzzes the append hot path) and
//! `fuzz_recovery` (which fuzzes open over arbitrary directory contents).
//!
//! The invariant checked on every `for_each_from` call:
//!
//! 1. Never panics (survival to the end of the harness = pass).
//! 2. The `seq → item` mapping is correct: since we always append
//!    `next_sequence` as the item value, `seq == item` must hold for every
//!    visited pair.
//! 3. Items are strictly ascending within a single call.
//!
//! An `Err` return from `for_each_from` is acceptable — it mirrors the
//! documented concurrent-delete race (segment removed by a prior
//! `delete_acked` between the scan and the per-segment read).
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_for_each_from
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use tempfile::tempdir;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    // Byte 0: n_setup_items (0..=255)
    // Byte 1: flush_interval (0 = Manual during setup, N = Batch(N) auto-flush)
    // Byte 2: start_seq (used directly as the for_each_from start, 0..=255)
    // Byte 3: limit_scale (limit = limit_scale * 4, so 0 → limit=0 edge case)
    // Remaining bytes: operation sequence (one byte per op)
    let n_setup_items = data[0] as usize;
    let flush_interval = data[1] as usize;
    let start_seq = data[2] as u64;
    let limit = (data[3] as usize).saturating_mul(4);

    let dir = tempdir().expect("tempdir must succeed");
    let config = if flush_interval == 0 {
        SegmentConfig::builder()
            .flush_policy(FlushPolicy::Manual)
            .build()
    } else {
        SegmentConfig::builder()
            .flush_at_batch_size(flush_interval)
            .build()
    };
    let buf = SegmentBuffer::<u64>::open(dir.path(), config).expect("open must succeed");

    // Setup: append n_setup_items items sequentially. Item i gets seq i.
    for i in 0..n_setup_items {
        let _ = buf.append(i as u64);
    }

    // Operation sequence: interleave for_each_from with mutations.
    let ops = &data[4..];
    for &op in ops {
        match op % 4 {
            0 => {
                // for_each_from: must never panic, seq must equal item, items
                // must be strictly ascending. An Err is acceptable (Io race).
                let mut prev: Option<u64> = None;
                let result = buf.for_each_from(start_seq, limit, |seq, item| {
                    assert_eq!(seq, *item, "seq→item mapping broken: seq={seq} item={item}");
                    if let Some(p) = prev {
                        assert!(*item > p, "out-of-order item: {item} after {p}");
                    }
                    prev = Some(*item);
                });
                let _ = result;
            }
            1 => {
                let _ = buf.flush();
            }
            2 => {
                // delete_acked at the midpoint — removes some on-disk segments.
                let latest = buf.latest_sequence();
                let _ = buf.delete_acked(latest / 2);
            }
            3 => {
                // Append one more item. Use next_sequence as the item value so
                // the seq→item invariant (seq == item) is preserved.
                let next = buf.stats().next_sequence;
                let _ = buf.append(next);
            }
            _ => {}
        }
    }
});
