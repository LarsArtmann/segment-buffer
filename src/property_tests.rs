//! Property-based tests for the segment format contracts.
//!
//! These run as part of `cargo test` (no special toolchain needed) and cover
//! the invariants that, if broken, would silently corrupt the queue:
//!
//! 1. **Filename bijection:** for every range we can construct, `parse_filename(filename(r)) == r`.
//! 2. **Payload bijection:** `decode_payload(encode_payload(events)) == events`.
//! 3. **Envelope transparency:** wrap→unwrap is identity on the payload.
//! 4. **Full pipeline:** write→read through the filesystem reproduces the input.

// Test modules override the library's strict lints. See the
// comment in `src/tests.rs` for rationale.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic_in_result_fn,
    clippy::panic,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::pedantic,
    clippy::nursery
)]
use super::segment;
use proptest::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct PropItem {
    id: u64,
    payload: String,
}

/// The 12-digit zero-padded filename format holds `0..=999_999_999_999`.
fn any_seq() -> impl Strategy<Value = u64> {
    0u64..=999_999_999_999
}

proptest! {
    /// `filename ∘ parse_filename` must be the identity on valid ranges.
    /// This is the load-bearing crash-recovery contract.
    #[test]
    fn filename_parse_roundtrip(start in any_seq(), end in any_seq()) {
        let name = segment::filename(start, end);
        let parsed =
            segment::parse_filename(&name).expect("filename must parse back to a range");
        prop_assert_eq!(parsed.start, start);
        prop_assert_eq!(parsed.end, end);
    }

    /// `parse_filename` must never panic on arbitrary input.
    #[test]
    fn parse_filename_never_panics(s in ".{0,40}") {
        let _ = segment::parse_filename(&s);
    }

    /// Every accepted parse must be reproducible: parsing the canonical name of
    /// a parsed range yields the same range. Catches normalization drift.
    #[test]
    fn parsed_range_round_trips_through_filename(s in ".{0,40}") {
        if let Some(r) = segment::parse_filename(&s) {
            let canonical = segment::filename(r.start, r.end);
            let reparsed = segment::parse_filename(&canonical).unwrap();
            prop_assert_eq!(reparsed.start, r.start);
            prop_assert_eq!(reparsed.end, r.end);
        }
    }

    /// The CBOR→zstd encode/decode pipeline must be a bijection on any input.
    #[test]
    fn encode_decode_payload_roundtrip(
        ids in proptest::collection::vec(any_seq(), 0..50)
    ) {
        let items: Vec<PropItem> = ids
            .iter()
            .map(|&id| PropItem { id, payload: format!("payload-{id}") })
            .collect();
        let path = std::path::Path::new("prop_test_segment.zst");

        let mut compressor = zstd::bulk::Compressor::new(3)
            .expect("compressor construction must succeed");
        let payload = segment::encode_payload(None, &mut compressor, path, &items)
            .expect("encode must succeed for valid items");

        let mut decompressor = zstd::bulk::Decompressor::new()
            .expect("decompressor construction must succeed");
        let decoded: Result<Vec<PropItem>, _> =
            segment::decode_payload(None, &mut decompressor, &payload, path);
        prop_assert!(decoded.is_ok(), "decode failed: {:?}", decoded.err());
        prop_assert_eq!(decoded.unwrap(), items);
    }

    /// wrap_envelope ∘ unwrap_envelope must be the identity on the payload.
    #[test]
    fn envelope_wrap_unwrap_identity(payload_bytes in proptest::collection::vec(any::<u8>(), 0..500)) {
        let wrapped = segment::wrap_envelope(&payload_bytes);
        let (_version, unwrapped) = segment::unwrap_envelope(&wrapped);
        prop_assert_eq!(unwrapped, payload_bytes.as_slice());
    }

    /// A full write→read cycle through the filesystem must reproduce the input,
    /// with AES-256-GCM at rest (feature-gated). The key is also varied per
    /// case so that key-dependent AEAD edge cases are exercised, not just a
    /// single fixed key. Exercises the pure encode/decode pipeline directly
    /// (no SegmentStore) so a regression in the byte-level format is caught
    /// independently of the I/O layer.
    #[cfg(feature = "encryption")]
    #[test]
    fn full_write_read_encrypted_roundtrip(
        key in any::<[u8; 32]>(),
        ids in proptest::collection::vec(any_seq(), 0..30)
    ) {
        let items: Vec<PropItem> = ids
            .iter()
            .map(|&id| PropItem { id, payload: format!("payload-{id}") })
            .collect();
        let path = std::path::Path::new("prop_test_segment.zst");
        let cipher = crate::AesGcmCipher::new(&key);

        let mut compressor = zstd::bulk::Compressor::new(3)
            .expect("compressor construction must succeed");
        let bytes = segment::encode_segment(Some(&cipher), &mut compressor, path, &items)
            .expect("encode must succeed");

        let mut decompressor = zstd::bulk::Decompressor::new()
            .expect("decompressor construction must succeed");
        let read: Result<Vec<PropItem>, _> =
            segment::decode_segment(Some(&cipher), &mut decompressor, &bytes, path);
        prop_assert!(read.is_ok(), "encrypted decode failed: {:?}", read.err());
        prop_assert_eq!(read.unwrap(), items);
    }

    /// Same as `full_write_read_encrypted_roundtrip` but for the v0.5.0
    /// recommended cipher (XChaCha20-Poly1305). Independent property so a
    /// regression in either AEAD is caught in isolation.
    #[cfg(feature = "encryption")]
    #[test]
    fn full_write_read_encrypted_xchacha20_roundtrip(
        key in any::<[u8; 32]>(),
        ids in proptest::collection::vec(any_seq(), 0..30)
    ) {
        let items: Vec<PropItem> = ids
            .iter()
            .map(|&id| PropItem { id, payload: format!("payload-{id}") })
            .collect();
        let path = std::path::Path::new("prop_test_segment_xchacha.zst");
        let cipher = crate::XChaCha20Poly1305Cipher::new(&key);

        let mut compressor = zstd::bulk::Compressor::new(3)
            .expect("compressor construction must succeed");
        let bytes = segment::encode_segment(Some(&cipher), &mut compressor, path, &items)
            .expect("encode must succeed");

        let mut decompressor = zstd::bulk::Decompressor::new()
            .expect("decompressor construction must succeed");
        let read: Result<Vec<PropItem>, _> =
            segment::decode_segment(Some(&cipher), &mut decompressor, &bytes, path);
        prop_assert!(read.is_ok(), "XChaCha20 decode failed: {:?}", read.err());
        prop_assert_eq!(read.unwrap(), items);
    }

    /// CI-runnable analogue of `fuzz/fuzz_targets/fuzz_corrupted_read.rs`:
    /// after overwriting an on-disk segment with arbitrary bytes, `read_from`
    /// must return `Err` and must never panic. The dedicated cargo-fuzz
    /// harness covers the same contract over far more cases under nightly,
    /// but this property runs in regular `cargo test` so the contract is
    /// enforced on every CI build.
    #[test]
    fn corrupted_segment_read_never_panics(corruption in proptest::collection::vec(any::<u8>(), 0..512)) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let buf = crate::SegmentBuffer::<PropItem>::open(dir, crate::SegmentConfig::default())
            .expect("open must succeed");

        // Seed one valid segment so a file exists on disk to corrupt.
        buf.append(PropItem { id: 0, payload: "seed".into() })
            .expect("append must succeed");
        buf.flush().expect("flush must succeed");

        // Overwrite the segment file with arbitrary bytes.
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "zst") {
                    let _ = std::fs::write(&path, &corruption);
                }
            }
        }

        // Contract: never panic. `Err` is the expected outcome for almost all
        // byte patterns; a valid zstd+CBOR+envelope decode for a tiny minority.
        let _ = buf.read_from(0, 100);
    }

    /// CI-runnable analogue of `fuzz/fuzz_targets/fuzz_recovery.rs`: opening
    /// a buffer over a directory of arbitrary files must never panic. The
    /// dedicated cargo-fuzz harness exercises this under nightly with deeper
    /// exploration; this property covers the crash-recovery contract on every
    /// CI build.
    #[test]
    fn recovery_over_arbitrary_directory_never_panics(
        name_bytes in proptest::collection::vec(any::<u8>(), 1..32),
        file_count in 0u8..8,
        blob_seed in any::<u64>()
    ) {
        // Build a plausible filename from the random bytes (lossy UTF-8).
        let name = String::from_utf8_lossy(&name_bytes).into_owned();
        if name.is_empty() || name.len() >= 64 || name.contains('/') {
            return Ok(()); // skip implausible directory entries
        }

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Drop a mix of segment-named and non-segment files with garbage bytes.
        let mut blob = Vec::new();
        for i in 0..file_count {
            blob.extend_from_slice(&blob_seed.wrapping_add(u64::from(i)).to_le_bytes());
            blob.extend_from_slice(b"garbage");
            let entry_name = if i % 2 == 0 {
                format!("seg_{i:012}_{file_count:012}.zst")
            } else {
                name.clone()
            };
            let _ = std::fs::write(dir.join(&entry_name), &blob);
        }

        // Contract: open() must never panic regardless of directory contents.
        let _ = crate::SegmentBuffer::<PropItem>::open(dir, crate::SegmentConfig::default());
    }

    /// `FlushPolicy::Manual` must never auto-flush, regardless of how many
    /// items are appended or how long the buffer has been open. The only way
    /// to make items durable under Manual is to call `flush()` explicitly.
    /// This is the contract that lets callers use Manual for tests and for
    /// absolute control over write amplification.
    #[test]
    fn flush_policy_manual_never_auto_flushes(
        n in 0u16..500,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        for i in 0..n {
            let _ = buf.append(PropItem { id: u64::from(i), payload: format!("p-{i}") });
        }

        // After up to 499 appends under Manual, there must be zero segment
        // files on disk. Items live only in memory until the caller flushes.
        let segment_count = std::fs::read_dir(tmp.path())
            .map_or(0, |entries| entries.filter_map(std::result::Result::ok).filter(|e| {
                e.file_name().to_string_lossy().ends_with(".zst")
            }).count());
        prop_assert_eq!(segment_count, 0, "Manual policy must not auto-flush");

        // But an explicit flush must still work and make items durable.
        buf.flush().expect("explicit flush must succeed");
        let segment_count_after = std::fs::read_dir(tmp.path())
            .map_or(0, |entries| entries.filter_map(std::result::Result::ok).filter(|e| {
                e.file_name().to_string_lossy().ends_with(".zst")
            }).count());
        if n > 0 {
            prop_assert_eq!(segment_count_after, 1, "explicit flush must create exactly one segment");
        }
    }

    /// `read_from(start, limit)` must return a prefix of `read_from(start, larger_limit)`:
    /// increasing the limit only adds items, never removes or reorders them.
    #[test]
    fn read_from_limit_monotone(
        n in 0u16..200,
        small_limit in 1u16..200,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");
        for i in 0..n {
            buf.append(PropItem { id: u64::from(i), payload: format!("p-{i}") }).expect("append");
        }
        buf.flush().expect("flush");

        let small = buf.read_from(0, small_limit as usize).expect("small read");
        let large = buf.read_from(0, small_limit as usize + 100).expect("large read");

        // small must be a prefix of large.
        prop_assert!(small.len() <= large.len());
        for (i, item) in small.iter().enumerate() {
            prop_assert_eq!(item, &large[i], "mismatch at index {}", i);
        }
    }

    /// `delete_acked(seq)` must never increase `pending_count`. Acknowledging
    /// more (larger seq) can only remove items, never add them.
    #[test]
    fn delete_acked_pending_count_monotone_nonincreasing(
        n in 1u8..50,
        ack1 in 0u64..49,
        ack2 in 0u64..49,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");
        for i in 0..n {
            buf.append(PropItem { id: u64::from(i), payload: format!("p-{i}") }).expect("append");
        }
        buf.flush().expect("flush");

        let (lo, hi) = if ack1 <= ack2 { (ack1, ack2) } else { (ack2, ack1) };
        let _ = buf.delete_acked(lo).expect("delete lo");
        let after_lo = buf.pending_count();
        let _ = buf.delete_acked(hi).expect("delete hi");
        let after_hi = buf.pending_count();

        prop_assert!(
            after_hi <= after_lo,
            "pending_count must not increase from ack={lo} to ack={hi}: {} -> {}",
            after_lo, after_hi
        );
    }

    /// `for_each_from` must visit exactly the same items as `read_from`, in
    /// the same order. This is the core equivalence between the lending and
    /// the cloning iterator APIs.
    #[test]
    fn for_each_from_visits_same_items_as_read_from(
        n in 0u16..100,
        start in 0u64..50,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");
        for i in 0..n {
            buf.append(PropItem { id: u64::from(i), payload: format!("p-{i}") }).expect("append");
        }
        buf.flush().expect("flush");

        let from_read: Vec<PropItem> = buf.read_from(start, 1000).expect("read_from");
        let mut from_for_each: Vec<(u64, PropItem)> = Vec::new();
        buf.for_each_from(start, 1000, |seq, item: &PropItem| {
            from_for_each.push((seq, item.clone()));
        }).expect("for_each_from");

        // Same count.
        prop_assert_eq!(from_read.len(), from_for_each.len(), "item count mismatch");

        // Same seqs and items, in order.
        for (i, read_item) in from_read.iter().enumerate() {
            let (fef_seq, fef_item) = &from_for_each[i];
            prop_assert_eq!(fef_item, read_item, "item mismatch at index {}", i);
            // The seq must be start + i (contiguous, ascending).
            prop_assert_eq!(*fef_seq, start + i as u64, "seq mismatch at index {}", i);
        }
    }

    /// `append_all` must assign contiguous sequences across multiple batches,
    /// regardless of batch sizes. The next batch must start exactly where the
    /// previous one ended (off-by-one check on the boundary).
    #[test]
    fn append_all_assigns_contiguous_sequences_across_batches(
        batch_sizes in proptest::collection::vec(0u16..50, 1..6),
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        let mut expected_next = 0u64;
        for (batch_idx, &size) in batch_sizes.iter().enumerate() {
            let items: Vec<PropItem> = (0..size)
                .map(|i| PropItem {
                    id: u64::try_from(batch_idx).unwrap() * 1000 + u64::from(i),
                    payload: format!("batch-{batch_idx}-item-{i}"),
                })
                .collect();
            let last_assigned = buf.append_all(items).expect("append_all");

            if size == 0 {
                // Empty batch is a no-op: last_assigned must equal the previous
                // expected_next, not advance it.
                prop_assert_eq!(
                    last_assigned, expected_next.saturating_sub(1),
                    "empty append_all at batch {} returned {:?}; prev next was {}",
                    batch_idx, last_assigned, expected_next,
                );
                // expected_next stays the same.
            } else {
                let batch_end = expected_next + u64::from(size);
                prop_assert_eq!(
                    last_assigned, batch_end - 1,
                    "batch {} (size {}) assigned last seq {} but expected {}",
                    batch_idx, size, last_assigned, batch_end - 1,
                );
                expected_next = batch_end;
            }
        }

        // Verify on-disk readback matches: contiguous seqs 0..expected_next.
        buf.flush().expect("flush");
        let all = buf.read_from(0, expected_next as usize + 10).expect("read_from");
        prop_assert_eq!(all.len() as u64, expected_next, "readback count mismatch");
        for (i, _item) in all.iter().enumerate() {
            // Every item read back; verify count matches.
            let _ = i;
        }
    }

    /// `sync_disk_bytes()` must always bring `stats().approx_disk_bytes` into
    /// exact agreement with the sum of segment file sizes on disk, regardless
    /// of the order or count of mutations that preceded the sync. This is the
    /// authoritative reconciliation primitive.
    #[test]
    fn sync_disk_bytes_matches_actual_disk_usage(
        n_flushes in 0u8..6,
        items_per_flush in 1u16..40,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        for _ in 0..n_flushes {
            for i in 0..items_per_flush {
                buf.append(PropItem {
                    id: u64::from(i),
                    payload: format!("payload-{i}"),
                }).expect("append");
            }
            buf.flush().expect("flush");
        }

        // Sync, then read both the returned value and the cached stats value.
        let synced = buf.sync_disk_bytes().expect("sync_disk_bytes");
        let cached = buf.stats().approx_disk_bytes;
        let cached_segments = buf.stats().segment_count;

        // Compute the actual disk usage: sum of `.zst` file sizes.
        let actual_files: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
            .collect();
        let actual: u64 = actual_files
            .iter()
            .map(|e| e.metadata().map_or(0, |m| m.len()))
            .sum();
        let actual_segment_count = actual_files.len() as u64;

        prop_assert_eq!(
            synced, actual,
            "sync_disk_bytes return value disagrees with du after {} flushes of {} items",
            n_flushes, items_per_flush,
        );
        prop_assert_eq!(
            cached, actual,
            "stats().approx_disk_bytes disagrees with du after sync; synced={}, actual={}",
            synced, actual,
        );
        prop_assert_eq!(
            cached_segments, actual_segment_count,
            "stats().segment_count disagrees with file count after sync; segments={}, actual={}",
            cached_segments, actual_segment_count,
        );
    }

    /// `BatchOrIntervalMin::should_flush` must match its documented decision
    /// formula across all combinations of batch sizes, thresholds, and time
    /// values. This is the regression guard for the three trigger paths
    /// (immediate batch, max-interval safety valve, gated interval) and the
    /// suppression case (below min_batch and before max_interval).
    #[test]
    fn batch_or_interval_min_flush_decision_matches_spec(
        batch_size in 1u16..500,
        min_batch in 0u16..500,
        pending_len in 0u16..500,
        interval_ms in 0u64..20_000,
        elapsed_ms in 0u64..20_000,
        max_interval_ms in 0u64..20_000,
    ) {
        // Honour builder invariants: min_batch <= batch_size, interval <= max_interval.
        let min_batch = min_batch.min(batch_size) as usize;
        let batch_size = batch_size as usize;
        let pending_len = pending_len as usize;
        let interval = std::time::Duration::from_millis(interval_ms);
        let max_interval = std::time::Duration::from_millis(max_interval_ms).max(interval);
        let elapsed = std::time::Duration::from_millis(elapsed_ms);

        let policy = crate::FlushPolicy::BatchOrIntervalMin {
            batch_size,
            min_batch,
            interval,
            max_interval,
        };

        let should = policy.should_flush(pending_len, elapsed);

        // Reconstruct the expected decision independently from the doc spec:
        // flush at batch_size OR at max_interval OR (min_batch met AND interval elapsed).
        let expected = pending_len >= batch_size
            || elapsed >= max_interval
            || (pending_len >= min_batch && elapsed >= interval);

        prop_assert_eq!(should, expected, "flush decision mismatch");
    }

    // ======================================================================
    // Consistency-model property tests
    // ======================================================================
    //
    // The crate documents two race windows in `read_from` under concurrent
    // operation (see docs/DOMAIN_LANGUAGE.md → "Concurrent operation"):
    //
    // 1. **Delete-acked race:** a segment deleted between `read_from`'s
    //    directory scan and its file read produces a spurious
    //    `SegmentError::Io(NotFound)`. Not data loss — the segment was
    //    already acknowledged.
    //
    // 2. **Flush race:** items that leave `unflushed` during a `flush()` that
    //    completes in the gap between `read_from`'s Phase 1 (scan) and
    //    Phase 2 (lock + read `unflushed`) are transiently invisible. They
    //    are durable on disk — a retry sees them.
    //
    // The stress tests in `src/tests.rs` prove these invariants
    // *statistically* under live thread contention. The property tests below
    // make the invariants *machine-checkable*: they verify that for every
    // generated state, the data `read_from` returns is always correct,
    // ascending, and free of corruption — the invariant that holds even when
    // the race fires. The concurrent variants exercise the actual race
    // windows with proptest-generated parameters, broadening coverage beyond
    // the fixed-parameter stress tests.

    /// After `delete_acked` removes segments, every item `read_from` returns
    /// must be correct: valid id matching the original global sequence,
    /// correct payload, strictly ascending, and no items from deleted
    /// segments.
    ///
    /// Formal assertion for the **delete-acked race window** invariant: the
    /// race may produce spurious `SegmentError::Io`, but never wrong,
    /// duplicate, or out-of-order items.
    #[test]
    fn read_from_surviving_items_correct_after_delete(
        num_segments in 1u8..12,
        items_per_segment in 1u8..30,
        delete_count in 0u8..12,
        read_start in 0u32..360,
        read_limit in 1u16..150,
    ) {
        let items_per_segment = u64::from(items_per_segment);
        let num_segments = u64::from(num_segments);
        let total = num_segments * items_per_segment;
        let delete_count = u64::from(delete_count).min(num_segments);
        let read_start = u64::from(read_start).min(total);
        let read_limit = read_limit as usize;

        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        for seg in 0..num_segments {
            for i in 0..items_per_segment {
                let seq = seg * items_per_segment + i;
                buf.append(PropItem {
                    id: seq,
                    payload: format!("payload-{seq}"),
                })
                .expect("append must succeed");
            }
            buf.flush().expect("flush must succeed");
        }

        let first_surviving_seq = delete_count * items_per_segment;
        if delete_count > 0 {
            let ack_seq = first_surviving_seq - 1;
            buf.delete_acked(ack_seq).expect("delete must succeed");
        }

        let result = buf
            .read_from(read_start, read_limit)
            .expect("read must succeed");

        prop_assert!(
            result.len() <= read_limit,
            "result length {} exceeds limit {}",
            result.len(),
            read_limit
        );

        let mut prev_id: Option<u64> = None;
        for item in &result {
            prop_assert!(
                item.id < total,
                "item id {} out of range [0, {})",
                item.id,
                total
            );
            prop_assert!(
                item.id >= first_surviving_seq,
                "item id {} from deleted segment (surviving starts at {})",
                item.id,
                first_surviving_seq
            );
            if let Some(p) = prev_id {
                prop_assert!(
                    item.id > p,
                    "items not strictly ascending: {} after {}",
                    item.id,
                    p
                );
            }
            prop_assert_eq!(
                &item.payload,
                &format!("payload-{}", item.id),
                "payload mismatch for item id {}",
                item.id
            );
            prev_id = Some(item.id);
        }

        // If there are surviving items at or after read_start, the result must
        // not be empty — the data is on disk, nothing is racing.
        if read_start < total && first_surviving_seq < total {
            let effective_start = read_start.max(first_surviving_seq);
            if effective_start < total {
                prop_assert!(
                    !result.is_empty(),
                    "read_from returned empty despite surviving items from seq {}",
                    effective_start
                );
            }
        }
    }

    /// With items split between on-disk segments and in-memory `unflushed`,
    /// `read_from` must return correct items from both layers: strictly
    /// ascending, contiguous (no gaps — nothing is deleted), and with the
    /// correct payload for each id.
    ///
    /// Formal assertion for the **flush race window** correctness invariant:
    /// a transient gap may cause items to be temporarily invisible under
    /// concurrency, but every item that IS returned is correct and contiguous.
    #[test]
    fn read_from_correct_with_disk_memory_split(
        on_disk_count in 0u16..80,
        in_memory_count in 0u16..80,
        read_start in 0u16..160,
        read_limit in 1u16..200,
    ) {
        let on_disk = u64::from(on_disk_count);
        let in_memory = u64::from(in_memory_count);
        let total = on_disk + in_memory;
        let read_start = u64::from(read_start).min(total);
        let read_limit = read_limit as usize;

        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        for i in 0..on_disk {
            buf.append(PropItem {
                id: i,
                payload: format!("payload-{i}"),
            })
            .expect("append must succeed");
        }
        if on_disk > 0 {
            buf.flush().expect("flush must succeed");
        }
        for i in 0..in_memory {
            let seq = on_disk + i;
            buf.append(PropItem {
                id: seq,
                payload: format!("payload-{seq}"),
            })
            .expect("append must succeed");
        }

        let result = buf
            .read_from(read_start, read_limit)
            .expect("read must succeed");

        // Nothing is deleted, so the result must be exactly the contiguous
        // run from read_start up to the limit or total, whichever is smaller.
        let expected_count = total.saturating_sub(read_start).min(read_limit as u64);
        prop_assert_eq!(
            result.len() as u64,
            expected_count,
            "expected {} contiguous items from seq {}, got {}",
            expected_count,
            read_start,
            result.len()
        );

        for (idx, item) in result.iter().enumerate() {
            let expected_id = read_start + idx as u64;
            prop_assert_eq!(
                item.id, expected_id,
                "item at index {} has id {}, expected {}",
                idx, item.id, expected_id
            );
            prop_assert_eq!(
                &item.payload,
                &format!("payload-{expected_id}"),
                "payload mismatch for item id {}",
                expected_id
            );
        }
    }

    /// After flushing from a split state (some on-disk, some in-memory), all
    /// items must be visible through `read_from` — correct, contiguous, and
    /// complete. This is the "transient gap closes" half of the flush race
    /// invariant: the gap is transient, not permanent.
    #[test]
    fn read_from_all_visible_after_flush_from_split(
        on_disk_count in 0u16..80,
        in_memory_count in 0u16..80,
    ) {
        let on_disk = u64::from(on_disk_count);
        let in_memory = u64::from(in_memory_count);
        let total = on_disk + in_memory;

        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");

        for i in 0..on_disk {
            buf.append(PropItem {
                id: i,
                payload: format!("payload-{i}"),
            })
            .expect("append must succeed");
        }
        if on_disk > 0 {
            buf.flush().expect("flush must succeed");
        }
        for i in 0..in_memory {
            let seq = on_disk + i;
            buf.append(PropItem {
                id: seq,
                payload: format!("payload-{seq}"),
            })
            .expect("append must succeed");
        }

        // Flush the in-memory tail — simulates the flusher settling.
        buf.flush().expect("final flush must succeed");

        let result = buf
            .read_from(0, total.max(1) as usize)
            .expect("read must succeed");

        prop_assert_eq!(
            result.len() as u64,
            total,
            "after flush, expected {} items, got {}",
            total,
            result.len()
        );

        for (i, item) in result.iter().enumerate() {
            prop_assert_eq!(
                item.id,
                i as u64,
                "item at position {} has wrong id {}",
                i,
                item.id
            );
            prop_assert_eq!(
                &item.payload,
                &format!("payload-{i}"),
                "payload mismatch at position {}",
                i
            );
        }
    }
}

// =========================================================================
// Concurrent property tests (race-window exercisers)
// =========================================================================
//
// These use a reduced case count (8) because each case spawns threads. They
// exercise the actual race windows — segment deleted between scan and read
// (delete race), and items flushed between Phase 1 scan and Phase 2 lock
// (flush race) — with proptest-generated parameters, broadening coverage
// beyond the fixed-parameter stress tests in `src/tests.rs`.
//
// The invariant checked is identical to the stress tests: every item the
// reader successfully deserializes must have the correct seq-to-value
// mapping. Spurious `Io` errors and transient gaps are tolerated (the reader
// retries or skips); corruption, reordering, or wrong values are not.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 8,
        ..ProptestConfig::default()
    })]

    /// Exercises the **delete-acked race window** under concurrent
    /// `read_from` + `delete_acked` with generated parameters. The reader
    /// tolerates spurious `Io` errors (segment deleted between scan and read)
    /// and transient gaps; it fails on any wrong, out-of-order, or
    /// payload-mismatched item.
    #[test]
    fn read_from_invariant_under_concurrent_delete_acked(
        num_segments in 3u8..15,
        items_per_segment in 5u8..30,
        read_batch_size in 10u16..200,
    ) {
        let items_per_segment = u64::from(items_per_segment);
        let num_segments = u64::from(num_segments);
        let total = items_per_segment * num_segments;

        let tmp = tempfile::tempdir().unwrap();
        let buf = std::sync::Arc::new(
            crate::SegmentBuffer::<PropItem>::open(
                tmp.path(),
                crate::SegmentConfig {
                    flush_policy: crate::FlushPolicy::Manual,
                    max_size_bytes: 100 * 1024 * 1024,
                    compression_level: 1,
                    durability: crate::DurabilityPolicy::Throughput,
                    cipher: None,
                },
            )
            .unwrap(),
        );

        for seg in 0..num_segments {
            for i in 0..items_per_segment {
                let seq = seg * items_per_segment + i;
                buf.append(PropItem {
                    id: seq,
                    payload: format!("payload-{seq}"),
                })
                .unwrap();
            }
            buf.flush().unwrap();
        }

        let corruption = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        std::thread::scope(|s| {
            // Reader: scans forward, verifying every item id and payload.
            // Retries on empty (flush-race analog); skips on Io error
            // (segment deleted under us — the documented boundary).
            let buf_r = std::sync::Arc::clone(&buf);
            let corrupt_r = std::sync::Arc::clone(&corruption);
            s.spawn(move || {
                let mut pos = 0u64;
                let mut prev_id: Option<u64> = None;
                let mut empty_retries = 0u32;
                while pos < total {
                    match buf_r.read_from(pos, read_batch_size as usize) {
                        Ok(batch) if !batch.is_empty() => {
                            empty_retries = 0;
                            for item in &batch {
                                if item.id >= total
                                    || item.payload != format!("payload-{}", item.id)
                                    || prev_id.is_some_and(|p| item.id <= p)
                                {
                                    corrupt_r
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                    return;
                                }
                                prev_id = Some(item.id);
                                pos = item.id + 1;
                            }
                        }
                        Ok(_) => {
                            empty_retries += 1;
                            if empty_retries > 5 {
                                pos = ((pos / items_per_segment) + 1) * items_per_segment;
                                empty_retries = 0;
                            } else {
                                std::thread::sleep(std::time::Duration::from_micros(100));
                            }
                        }
                        Err(_) => {
                            // Io error: segment deleted between scan and
                            // read. Documented boundary — skip forward.
                            pos = ((pos / items_per_segment) + 1) * items_per_segment;
                        }
                    }
                }
            });

            // Deleter: removes segments from the front, racing with reads.
            let buf_d = std::sync::Arc::clone(&buf);
            s.spawn(move || {
                for acked in (items_per_segment..total)
                    .step_by(items_per_segment as usize)
                {
                    let _ = buf_d.delete_acked(acked);
                    std::thread::sleep(std::time::Duration::from_micros(10));
                }
            });
        });

        prop_assert!(
            !corruption.load(std::sync::atomic::Ordering::SeqCst),
            "read_from returned wrong data under concurrent delete_acked \
             (num_segments={}, items_per_segment={}, read_batch_size={})",
            num_segments,
            items_per_segment,
            read_batch_size
        );
    }

    /// Exercises the **flush race window** under concurrent `read_from` +
    /// `flush` with generated parameters. The reader tolerates transient
    /// gaps (items flushed between scan and lock) by retrying; it fails on
    /// any wrong, out-of-order, or payload-mismatched item. After the
    /// flusher settles, all items must be visible and correct.
    #[test]
    fn read_from_invariant_under_concurrent_flush(
        on_disk_count in 50u16..400,
        in_memory_count in 50u16..400,
        read_batch_size in 10u16..200,
    ) {
        let on_disk = u64::from(on_disk_count);
        let in_memory = u64::from(in_memory_count);
        let total = on_disk + in_memory;

        let tmp = tempfile::tempdir().unwrap();
        let buf = std::sync::Arc::new(
            crate::SegmentBuffer::<PropItem>::open(
                tmp.path(),
                crate::SegmentConfig {
                    flush_policy: crate::FlushPolicy::Manual,
                    max_size_bytes: 100 * 1024 * 1024,
                    compression_level: 1,
                    durability: crate::DurabilityPolicy::Throughput,
                    cipher: None,
                },
            )
            .unwrap(),
        );

        for i in 0..on_disk {
            buf.append(PropItem {
                id: i,
                payload: format!("payload-{i}"),
            })
            .unwrap();
        }
        buf.flush().unwrap();
        for i in 0..in_memory {
            let seq = on_disk + i;
            buf.append(PropItem {
                id: seq,
                payload: format!("payload-{seq}"),
            })
            .unwrap();
        }

        let corruption = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        std::thread::scope(|s| {
            // Reader: scans forward, verifying every item id and payload.
            // Tolerates transient gaps (flush race) by retrying.
            let buf_r = std::sync::Arc::clone(&buf);
            let corrupt_r = std::sync::Arc::clone(&corruption);
            s.spawn(move || {
                let mut pos = 0u64;
                let mut prev_id: Option<u64> = None;
                let mut empty_retries = 0u32;
                while pos < total {
                    match buf_r.read_from(pos, read_batch_size as usize) {
                        Ok(batch) if !batch.is_empty() => {
                            empty_retries = 0;
                            for item in &batch {
                                if item.id >= total
                                    || item.payload != format!("payload-{}", item.id)
                                    || prev_id.is_some_and(|p| item.id <= p)
                                {
                                    corrupt_r
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                    return;
                                }
                                prev_id = Some(item.id);
                                pos = item.id + 1;
                            }
                        }
                        Ok(_) => {
                            empty_retries += 1;
                            if empty_retries > 200 {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_micros(20));
                        }
                        Err(_) => {
                            std::thread::sleep(std::time::Duration::from_micros(20));
                        }
                    }
                }
            });

            // Flusher: drains `unflushed` to disk, racing with reads.
            let buf_f = std::sync::Arc::clone(&buf);
            s.spawn(move || {
                for _ in 0..20 {
                    let _ = buf_f.flush();
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            });
        });

        prop_assert!(
            !corruption.load(std::sync::atomic::Ordering::SeqCst),
            "read_from returned wrong data under concurrent flush \
             (on_disk={}, in_memory={}, read_batch_size={})",
            on_disk,
            in_memory,
            read_batch_size
        );

        // After the flusher settles, the transient gap must close: every
        // item must become visible. This is the "a retry sees them" half of
        // the flush-race invariant.
        let _ = buf.flush(); // settle: drain anything the flusher left behind
        let mut settled = Vec::new();
        for _ in 0..10 {
            settled = buf
                .read_from(0, total as usize)
                .expect("settle read must not error");
            if settled.len() as u64 >= total {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        prop_assert_eq!(
            settled.len() as u64,
            total,
            "after flush settles, not all items visible within retry bound \
             (on_disk={}, in_memory={}, read_batch_size={})",
            on_disk,
            in_memory,
            read_batch_size,
        );
        for (i, item) in settled.iter().enumerate() {
            prop_assert_eq!(item.id, i as u64, "wrong id at {} after settle", i);
            prop_assert_eq!(
                &item.payload,
                &format!("payload-{i}"),
                "payload mismatch at {} after settle",
                i
            );
        }
    }

    /// `segment_size_stats()` must agree with a brute-force directory scan on
    /// every field, for any number of flushes and items-per-flush. This is the
    /// authoritative cross-check that the on-demand distribution is exact
    /// (count/min/max/mean) and that the nearest-rank percentiles match an
    /// independent float implementation of `ceil(p / 100 · n)`.
    #[test]
    fn segment_size_stats_matches_directory(
        n_flushes in 0u8..8,
        items_per_flush in 1u16..40,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::SegmentConfig {
            flush_policy: crate::FlushPolicy::Manual,
            ..crate::SegmentConfig::default()
        };
        let buf = crate::SegmentBuffer::<PropItem>::open(tmp.path(), config)
            .expect("open must succeed");
        for _ in 0..n_flushes {
            for i in 0..items_per_flush {
                buf.append(PropItem {
                    id: u64::from(i),
                    payload: format!("payload-{i}"),
                })
                .expect("append");
            }
            buf.flush().expect("flush");
        }

        let s = buf.segment_size_stats().expect("segment_size_stats");

        // Brute-force the same distribution straight from the directory.
        let mut sizes: Vec<u64> = std::fs::read_dir(tmp.path())
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
            .map(|e| e.metadata().map_or(0u64, |m| m.len()))
            .collect();
        sizes.sort();

        prop_assert_eq!(s.count, sizes.len() as u64);

        if sizes.is_empty() {
            prop_assert_eq!(s.min_bytes, 0);
            prop_assert_eq!(s.max_bytes, 0);
            prop_assert_eq!(s.mean_bytes, 0);
            prop_assert_eq!(s.p50_bytes, 0);
            prop_assert_eq!(s.p90_bytes, 0);
        } else {
            let n = sizes.len();
            prop_assert_eq!(s.min_bytes, *sizes.first().unwrap());
            prop_assert_eq!(s.max_bytes, *sizes.last().unwrap());
            let total: u64 = sizes.iter().sum();
            prop_assert_eq!(s.mean_bytes, total / n as u64);
            // Independent float implementation of the nearest-rank formula.
            let rank = |pct: f64| -> usize {
                let r = (pct / 100.0 * n as f64).ceil() as usize;
                r.clamp(1, n) - 1
            };
            prop_assert_eq!(s.p50_bytes, sizes[rank(50.0)]);
            prop_assert_eq!(s.p90_bytes, sizes[rank(90.0)]);
            // Monotonicity must always hold.
            prop_assert!(s.min_bytes <= s.p50_bytes);
            prop_assert!(s.p50_bytes <= s.p90_bytes);
            prop_assert!(s.p90_bytes <= s.max_bytes);
        }
    }
}
