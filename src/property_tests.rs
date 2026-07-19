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
    /// with AES-256-GCM at rest (feature-gated).
    #[cfg(feature = "encryption")]
    #[test]
    fn full_write_read_encrypted_roundtrip(
        ids in proptest::collection::vec(any_seq(), 0..30)
    ) {
        let items: Vec<PropItem> = ids
            .iter()
            .map(|&id| PropItem { id, payload: format!("payload-{id}") })
            .collect();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let cipher = crate::AesGcmCipher::new(&[0x42u8; 32]);
        let end = items.len().saturating_sub(1) as u64;
        let range = segment::SegmentRange { start: 0, end };

        segment::write(dir, Some(&cipher), 3, range, &items)
            .expect("write must succeed");

        let read: Result<Vec<PropItem>, _> = segment::read(dir, Some(&cipher), range);
        prop_assert!(read.is_ok(), "encrypted read failed: {:?}", read.err());
        prop_assert_eq!(read.unwrap(), items);
    }
}
