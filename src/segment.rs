//! On-disk segment format: filename layout, encode/decode pipeline, scanning.
//!
//! A segment file is named `seg_{start:012}_{end:012}.zst` and contains
//! CBOR-serialized events, zstd-compressed, and optionally encrypted. This
//! module owns the byte-level format so that [`crate::SegmentBuffer`] can focus
//! on in-memory orchestration and locking.
//!
//! The filename format is a load-bearing contract: it is the *only* state used
//! for crash recovery, and it must stay byte-compatible with existing
//! monitor365 segment files. See [`filename`] / [`parse_filename`].

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::cipher::SegmentCipher;
use crate::error::{Result, SegmentError};

/// Filename prefix for every segment file.
const SEGMENT_PREFIX: &str = "seg_";
/// Filename suffix for finalized, zstd-compressed segment files.
const SEGMENT_SUFFIX: &str = ".zst";
/// Suffix for in-progress writes, treated as crash debris on recovery.
pub(super) const TMP_SUFFIX: &str = ".tmp";
/// Bytes of the AEAD nonce prefix written by [`SegmentCipher`] implementations
/// such as AES-256-GCM. Ciphertexts shorter than this cannot be valid.
const NONCE_LEN: usize = 12;

/// Inclusive `[start, end]` range of sequence numbers stored in a segment file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SegmentRange {
    /// First sequence number in the segment (inclusive).
    pub(crate) start: u64,
    /// Last sequence number in the segment (inclusive).
    pub(crate) end: u64,
}

/// Build the segment filename for an inclusive `[start, end]` range.
pub(crate) fn filename(start: u64, end: u64) -> String {
    format!("{SEGMENT_PREFIX}{start:012}_{end:012}{SEGMENT_SUFFIX}")
}

/// Parse `seg_{start:012}_{end:012}.zst` into a [`SegmentRange`].
///
/// Returns `None` for any name that is not a segment file, so callers can use
/// this to filter directory listings.
pub(crate) fn parse_filename(name: &str) -> Option<SegmentRange> {
    let core = name
        .strip_prefix(SEGMENT_PREFIX)?
        .strip_suffix(SEGMENT_SUFFIX)?;
    let (start_str, end_str) = core.split_once('_')?;
    let start = start_str.parse().ok()?;
    let end = end_str.parse().ok()?;
    Some(SegmentRange { start, end })
}

/// Scan `dir` and return every segment range found, sorted by `start`.
pub(crate) fn scan(dir: &Path) -> Result<Vec<SegmentRange>> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(range) = parse_filename(&entry.file_name().to_string_lossy()) {
            segments.push(range);
        }
    }
    segments.sort_by_key(|s| s.start);
    Ok(segments)
}

/// Delete leftover `*.tmp` files left behind by a crashed write.
///
/// Removal of individual files is best-effort (per-file errors are ignored);
/// only directory-read errors propagate. This mirrors the crash-recovery
/// contract: a half-written segment must not survive recovery.
pub(crate) fn clean_tmp(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(TMP_SUFFIX))
        {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Encode `events` to bytes: CBOR → zstd → optional encrypt.
///
/// This is the inverse of [`decode`].
fn encode<T: Serialize>(
    cipher: Option<&dyn SegmentCipher>,
    level: i32,
    events: &[T],
) -> Result<Vec<u8>> {
    let mut cbor_buf = Vec::new();
    ciborium::into_writer(events, &mut cbor_buf)
        .map_err(|e| SegmentError::Cbor(format!("serialization: {e}")))?;

    let compressed = zstd::encode_all(cbor_buf.as_slice(), level)?;

    match cipher {
        Some(cipher) => cipher.encrypt(&compressed),
        None => Ok(compressed),
    }
}

/// Decode bytes back to events: optional decrypt → zstd → CBOR.
///
/// This is the inverse of [`encode`].
fn decode<T: DeserializeOwned>(cipher: Option<&dyn SegmentCipher>, raw: Vec<u8>) -> Result<Vec<T>> {
    let compressed = match cipher {
        Some(cipher) => cipher.decrypt(&raw)?,
        None => raw,
    };

    let cbor_buf = zstd::decode_all(compressed.as_slice())?;
    ciborium::from_reader(cbor_buf.as_slice())
        .map_err(|e| SegmentError::Cbor(format!("deserialization: {e}")))
}

/// Write `events` for `range` to a segment file in `dir`.
///
/// Bytes are written to a `.tmp` sidecar, `sync_all`'d, then atomically renamed
/// to the final segment path so a crash never leaves a partial segment. Returns
/// the number of bytes written.
pub(crate) fn write<T: Serialize>(
    dir: &Path,
    cipher: Option<&dyn SegmentCipher>,
    level: i32,
    range: SegmentRange,
    events: &[T],
) -> Result<u64> {
    let final_bytes = encode(cipher, level, events)?;
    let seg_name = filename(range.start, range.end);
    let seg_path = dir.join(&seg_name);
    let tmp_path = dir.join(format!("{seg_name}{TMP_SUFFIX}"));

    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&final_bytes)?;
        file.sync_all()?;
    }

    fs::rename(&tmp_path, &seg_path)?;
    Ok(final_bytes.len() as u64)
}

/// Read and decode the segment file for `range` from `dir`.
///
/// Encrypted segments shorter than the AEAD nonce are rejected as
/// [`SegmentError::Integrity`] with the offending path, before the cipher is
/// invoked.
pub(crate) fn read<T: DeserializeOwned>(
    dir: &Path,
    cipher: Option<&dyn SegmentCipher>,
    range: SegmentRange,
) -> Result<Vec<T>> {
    let path = dir.join(filename(range.start, range.end));
    let raw = fs::read(&path)?;

    if cipher.is_some() && raw.len() < NONCE_LEN {
        return Err(SegmentError::Integrity(format!(
            "segment {} too small for nonce ({} bytes)",
            path.display(),
            raw.len()
        )));
    }

    decode(cipher, raw)
}
