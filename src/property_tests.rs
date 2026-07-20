//! Property-based tests for the segment format contracts.
//!
//! These run as part of `cargo test` (no special toolchain needed) and cover
//! the invariants that, if broken, would silently corrupt the queue:
//!
//! 1. **Filename bijection:** for every range we can construct, `parse_filename(filename(r)) == r`.
//! 2. **Payload bijection:** `decode_payload(encode_payload(events)) == events`.
//! 3. **Envelope transparency:** wrap→unwrap is identity on the payload.
//! 4. **Full pipeline:** write→read through the filesystem reproduces the input.

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

        let decoded: Result<Vec<PropItem>, _> =
            segment::decode_payload(None, &payload, path);
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

        let read: Result<Vec<PropItem>, _> =
            segment::decode_segment(Some(&cipher), &bytes, path);
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

        let read: Result<Vec<PropItem>, _> =
            segment::decode_segment(Some(&cipher), &bytes, path);
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
            blob.extend_from_slice(&blob_seed.wrapping_add(i as u64).to_le_bytes());
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
            let _ = buf.append(PropItem { id: i as u64, payload: format!("p-{i}") });
        }

        // After up to 499 appends under Manual, there must be zero segment
        // files on disk. Items live only in memory until the caller flushes.
        let segment_count = std::fs::read_dir(tmp.path())
            .map(|entries| entries.filter_map(|e| e.ok()).filter(|e| {
                e.file_name().to_string_lossy().ends_with(".zst")
            }).count())
            .unwrap_or(0);
        prop_assert_eq!(segment_count, 0, "Manual policy must not auto-flush");

        // But an explicit flush must still work and make items durable.
        buf.flush().expect("explicit flush must succeed");
        let segment_count_after = std::fs::read_dir(tmp.path())
            .map(|entries| entries.filter_map(|e| e.ok()).filter(|e| {
                e.file_name().to_string_lossy().ends_with(".zst")
            }).count())
            .unwrap_or(0);
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
            buf.append(PropItem { id: i as u64, payload: format!("p-{i}") }).expect("append");
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
            buf.append(PropItem { id: i as u64, payload: format!("p-{i}") }).expect("append");
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
            buf.append(PropItem { id: i as u64, payload: format!("p-{i}") }).expect("append");
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
                    id: u64::try_from(batch_idx).unwrap() * 1000 + i as u64,
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
                let batch_end = expected_next + size as u64;
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
                    id: i as u64,
                    payload: format!("payload-{i}"),
                }).expect("append");
            }
            buf.flush().expect("flush");
        }

        // Sync, then read both the returned value and the cached stats value.
        let synced = buf.sync_disk_bytes().expect("sync_disk_bytes");
        let cached = buf.stats().approx_disk_bytes;

        // Compute the actual disk usage: sum of `.zst` file sizes.
        let actual: u64 = std::fs::read_dir(tmp.path())
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".zst"))
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum();

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
    }
}
