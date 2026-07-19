//! Fuzz target: `append_all` must assign contiguous sequences under
//! arbitrary iterator behavior (empty, single, many, with arbitrary
//! payloads) and never panic.
//!
//! The iterator is synthesized from the fuzz input: the first byte selects
//! the batch size (0..=255), and the remaining bytes are split into that
//! many `u64` items (truncated if too short, zero-padded if too long).
//! This exercises:
//!
//! - Empty iterator (size = 0)
//! - Single-item batch
//! - Large batch with arbitrary size
//! - The flush-policy boundary (`Manual` so no I/O on the hot path)
//!
//! Invariants asserted on every iteration:
//!
//! 1. `append_all` never panics.
//! 2. The returned last seq equals `next_sequence - 1` after the call
//!    (off-by-one check on the sequence-assignment boundary).
//! 3. `pending_count` increased by exactly the number of items in the batch.
//! 4. A second `append_all` of an empty iterator is a no-op: it returns the
//!    same last seq as the prior call and does not change `pending_count`.
//!
//! ```sh
//! cargo +nightly fuzz run fuzz_append_all
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use segment_buffer::{FlushPolicy, SegmentBuffer, SegmentConfig};
use tempfile::tempdir;

fuzz_target!(|data: &[u8]| {
    // First byte = batch size (0..=255). Rest = payload bytes split into u64s.
    let (size_byte, rest) = data.split_first().unwrap_or((&0, &[]));
    let batch_size = *size_byte as usize;

    // Synthesize up to `batch_size` u64 items from the remaining bytes.
    // If bytes run out, we pad with zeros (keeps the iterator size fixed).
    let items: Vec<u64> = (0..batch_size)
        .map(|i| {
            let start = i * 8;
            if start + 8 <= rest.len() {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&rest[start..start + 8]);
                u64::from_le_bytes(buf)
            } else {
                0
            }
        })
        .collect();

    let dir = tempdir().expect("tempdir must succeed");
    let config = SegmentConfig::builder()
        .flush_policy(FlushPolicy::Manual)
        .build();
    let buf = SegmentBuffer::<u64>::open(dir.path(), config).expect("open must succeed");

    let prev_pending = buf.pending_count();
    let prev_last = buf.latest_sequence();

    let last_seq = buf
        .append_all(items.iter().copied())
        .expect("append_all must succeed");

    // Invariant 1: never panics — survival to here means it held.

    // Invariant 2: pending_count grew by exactly batch_size.
    let pending_after = buf.pending_count();
    assert_eq!(
        pending_after.saturating_sub(prev_pending),
        batch_size as u64,
        "pending_count grew by {} but batch_size was {batch_size}",
        pending_after.saturating_sub(prev_pending),
    );

    // Invariant 3: last_seq advances by exactly (batch_size - 1) for non-empty
    // batches (last = prev_last + batch_size - 1 + 1 = prev_last + batch_size,
    // so last_seq - prev_last == batch_size when prev_pending > 0). For the
    // first non-empty batch on an empty buffer, last_seq == batch_size - 1.
    if batch_size > 0 {
        if prev_pending == 0 {
            // First batch on empty buffer: seqs are 0..batch_size.
            assert_eq!(
                last_seq,
                (batch_size - 1) as u64,
                "first batch last_seq should be batch_size - 1"
            );
        } else {
            // Subsequent batch: last_seq advances by exactly batch_size.
            assert_eq!(
                last_seq,
                prev_last + batch_size as u64,
                "subsequent batch last_seq should advance by batch_size"
            );
        }
    } else {
        // Invariant 4: empty iterator is a no-op. last_seq == prev_last.
        assert_eq!(
            last_seq, prev_last,
            "empty append_all must return same last_seq as before"
        );
    }

    // Invariant 4 (second half): a follow-up empty append_all is a no-op.
    let last_seq_2 = buf
        .append_all(std::iter::empty::<u64>())
        .expect("empty append_all");
    assert_eq!(
        last_seq_2, last_seq,
        "empty append_all after non-empty must return same last_seq"
    );
    assert_eq!(
        buf.pending_count(),
        pending_after,
        "empty append_all must not change pending_count"
    );
});
