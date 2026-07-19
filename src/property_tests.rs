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

        let payload = segment::encode_payload(None, 3, path, &items)
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
    /// single fixed key.
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
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let cipher = crate::AesGcmCipher::new(&key);
        let end = items.len().saturating_sub(1) as u64;
        let range = segment::SegmentRange { start: 0, end };

        segment::write(dir, Some(&cipher), 3, range, &items)
            .expect("write must succeed");

        let read: Result<Vec<PropItem>, _> = segment::read(dir, Some(&cipher), range);
        prop_assert!(read.is_ok(), "encrypted read failed: {:?}", read.err());
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
}
