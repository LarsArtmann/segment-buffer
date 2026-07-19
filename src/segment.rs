//! On-disk segment format: filename layout, encode/decode pipeline, scanning.
//!
//! A segment file is named `seg_{start:012}_{end:012}.zst` and contains an
//! optional envelope header followed by the CBOR-serialized, zstd-compressed,
//! optionally-encrypted events. This module owns the byte-level format so that
//! [`crate::SegmentBuffer`] can focus on in-memory orchestration and locking.
//!
//! ## Envelope (format evolution)
//!
//! Every segment written by this crate is wrapped in an 8-byte envelope:
//!
//! ```text
//! offset  bytes   meaning
//! ------  -----   -------
//!   0..4    4     magic: ASCII `SBF1` ("Segment Buffer Format")
//!   4       1     envelope version (currently 1)
//!   5..8    3     reserved (all zero; future: checksum type, compression algo…)
//!   8..           payload (the v1 bytes: zstd(CBOR(events)), optionally encrypted)
//! ```
//!
//! The payload is byte-identical to the legacy v1 format, so the cipher still
//! sees `[nonce][ciphertext]` exactly as before — the envelope is stripped
//! before decryption.
//!
//! ## Legacy compatibility
//!
//! Files without the magic prefix are read as legacy v1 (the original
//! monitor365 format). This makes the envelope a strictly additive change:
//! existing segment files keep reading without migration, and new writes are
//! forward-compatible with future format versions. Detection requires the
//! `SBF1` magic **and** the 3 reserved bytes at offset `5..8` to all be zero,
//! so the false-positive rate on a legacy encrypted file (whose first 7 bytes
//! are random AEAD nonce) is 2⁻⁵⁶ per file — negligible even across the full
//! 597M-segment monitor365 corpus.
//!
//! The filename format is a load-bearing contract: it is the *only* state used
//! for crash recovery, and existing monitor365 filenames must still parse. See
//! [`filename`] / [`parse_filename`].

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
const TMP_SUFFIX: &str = ".tmp";
/// Bytes of the AEAD nonce prefix written by [`SegmentCipher`] implementations
/// such as AES-256-GCM. Ciphertexts shorter than this cannot be valid.
const NONCE_LEN: usize = 12;

/// Envelope magic: ASCII `SBF1` ("Segment Buffer Format"). Chosen to be
/// distinct from the zstd frame magic (`28 B5 2F FD`) and human-readable in a
/// hex dump.
const ENVELOPE_MAGIC: [u8; 4] = *b"SBF1";
/// Current envelope version. Version 1 = the original payload layout
/// (zstd(CBOR), optionally `[nonce][ciphertext]`).
const ENVELOPE_VERSION: u8 = 1;
/// Total envelope length: 4 magic + 1 version + 3 reserved.
const ENVELOPE_LEN: usize = 8;

/// Inclusive `[start, end]` range of sequence numbers stored in a segment file.
#[derive(Debug, Clone, Copy)]
pub struct SegmentRange {
    /// First sequence number in the segment (inclusive).
    pub start: u64,
    /// Last sequence number in the segment (inclusive).
    pub end: u64,
}

impl SegmentRange {
    /// Construct a segment range, asserting `start <= end` in debug builds.
    ///
    /// Crash recovery requires every on-disk filename to encode a valid
    /// inclusive range. Filenames in the wild should always honour that, but
    /// the buffer itself must never produce an inverted range: this constructor
    /// makes that a debug-time invariant. Parse-time validation stays loose
    /// (see [`parse_filename`]) because legacy files could in principle violate
    /// it, and we want to surface them rather than silently drop them.
    pub(crate) fn new(start: u64, end: u64) -> Self {
        debug_assert!(
            start <= end,
            "SegmentRange invariant violated: start ({start}) > end ({end})"
        );
        Self { start, end }
    }
}

/// Build the segment filename for an inclusive `[start, end]` range.
pub fn filename(start: u64, end: u64) -> String {
    format!("{SEGMENT_PREFIX}{start:012}_{end:012}{SEGMENT_SUFFIX}")
}

/// Parse `seg_{start:012}_{end:012}.zst` into a [`SegmentRange`].
///
/// Returns `None` for any name that is not a segment file, so callers can use
/// this to filter directory listings. Note: the format does not enforce
/// `start <= end` at the parse level (legacy files in the wild may violate it),
/// so callers that need the invariant must check.
pub fn parse_filename(name: &str) -> Option<SegmentRange> {
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
pub(crate) fn clean_tmp(dir: &Path) -> Result<usize> {
    let mut removed = 0usize;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(TMP_SUFFIX))
            && fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Reserved bytes at envelope offset `5..8`. Always zero in envelope v1;
/// future versions may repurpose them (checksum type, compression algo, …).
/// Required to be zero on read so the magic-only false-positive rate on
/// legacy encrypted files drops from 2⁻³² (4 random nonce bytes colliding
/// with `SBF1`) to 2⁻⁵⁶ (7 random bytes colliding with `SBF1\x00\x00\x00`),
/// making the legacy-compatibility guarantee actually hold across the full
/// 597M-segment monitor365 corpus.
const ENVELOPE_RESERVED_LEN: usize = 3;

/// Strip the envelope, if present, returning `(version, payload)`.
///
/// Returns `(Some(version), payload_after_envelope)` when the magic matches
/// **and** the 3 reserved bytes are all zero (the v1 layout invariants);
/// `(None, original_bytes)` for legacy v1 files. The payload is what the
/// cipher and zstd/CBOR layers operate on; it is identical in layout to a
/// v1 file. Requiring the reserved bytes to be zero is what makes the
/// legacy-detection false-positive rate negligible (2⁻⁵⁶ per file).
pub fn unwrap_envelope(raw: &[u8]) -> (Option<u8>, &[u8]) {
    let reserved_range = ENVELOPE_MAGIC.len() + 1..ENVELOPE_LEN;
    let reserved_zero = [0u8; ENVELOPE_RESERVED_LEN];
    if raw.len() >= ENVELOPE_LEN
        && raw[..ENVELOPE_MAGIC.len()] == ENVELOPE_MAGIC
        && raw[reserved_range] == reserved_zero
    {
        (Some(raw[ENVELOPE_MAGIC.len()]), &raw[ENVELOPE_LEN..])
    } else {
        (None, raw)
    }
}

/// Prepend the envelope to `payload`.
pub fn wrap_envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ENVELOPE_LEN + payload.len());
    out.extend_from_slice(&ENVELOPE_MAGIC);
    out.push(ENVELOPE_VERSION);
    // 3 reserved bytes, all zero (future: checksum type, compression algo, …).
    out.extend_from_slice(&[0u8; ENVELOPE_LEN - ENVELOPE_MAGIC.len() - 1]);
    out.extend_from_slice(payload);
    out
}

/// Encode `events` to the v1 payload bytes: CBOR → zstd → optional encrypt.
pub(crate) fn encode_payload<T: Serialize>(
    cipher: Option<&dyn SegmentCipher>,
    level: i32,
    path: &Path,
    events: &[T],
) -> Result<Vec<u8>> {
    let mut cbor_buf = Vec::new();
    ciborium::into_writer(events, &mut cbor_buf).map_err(|e| SegmentError::Cbor {
        phase: "serialize",
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    let compressed = zstd::encode_all(cbor_buf.as_slice(), level)?;

    match cipher {
        Some(cipher) => cipher
            .encrypt(&compressed)
            .map_err(|e| SegmentError::Cipher {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
        None => Ok(compressed),
    }
}

/// Decode the v1 payload bytes back to events: optional decrypt → zstd → CBOR.
pub(crate) fn decode_payload<T: DeserializeOwned>(
    cipher: Option<&dyn SegmentCipher>,
    payload: &[u8],
    path: &Path,
) -> Result<Vec<T>> {
    // Decrypt into an owned buffer if a cipher is configured; otherwise borrow.
    // The `Cow` avoids cloning the (potentially large) plaintext zstd blob.
    use std::borrow::Cow;
    let decrypted;
    let compressed: Cow<[u8]> = match cipher {
        Some(cipher) => {
            decrypted = cipher.decrypt(payload).map_err(|e| SegmentError::Cipher {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            Cow::Owned(decrypted)
        }
        None => Cow::Borrowed(payload),
    };

    let cbor_buf = zstd::decode_all(compressed.as_ref())?;
    ciborium::from_reader(cbor_buf.as_slice()).map_err(|e| SegmentError::Cbor {
        phase: "deserialize",
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Write `events` for `range` to a segment file in `dir`.
///
/// Bytes are written to a `.tmp` sidecar, `sync_all`'d, then atomically renamed
/// to the final segment path so a crash never leaves a partial segment. The
/// file is prefixed with the 8-byte [`envelope`](self#envelope-format-evolution)
/// so the format can evolve without breaking legacy readers. Returns the number
/// of bytes written (envelope + payload).
pub(crate) fn write<T: Serialize>(
    dir: &Path,
    cipher: Option<&dyn SegmentCipher>,
    level: i32,
    range: SegmentRange,
    events: &[T],
) -> Result<u64> {
    let seg_name = filename(range.start, range.end);
    let seg_path = dir.join(&seg_name);
    let payload = encode_payload(cipher, level, &seg_path, events)?;
    let final_bytes = wrap_envelope(&payload);
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
/// Both enveloped (current) and legacy (pre-envelope, monitor365-compatible)
/// files are accepted; see the [`envelope`](self#envelope-format-evolution)
/// section. Encrypted payloads shorter than the AEAD nonce are rejected as
/// [`SegmentError::Integrity`] with the offending path, before the cipher is
/// invoked.
pub(crate) fn read<T: DeserializeOwned>(
    dir: &Path,
    cipher: Option<&dyn SegmentCipher>,
    range: SegmentRange,
) -> Result<Vec<T>> {
    let path = dir.join(filename(range.start, range.end));
    let raw = fs::read(&path)?;
    let (_version, payload) = unwrap_envelope(&raw);

    if cipher.is_some() && payload.len() < NONCE_LEN {
        return Err(SegmentError::Integrity {
            path,
            reason: "encrypted payload too small for AEAD nonce",
        });
    }

    decode_payload(cipher, payload, &path)
}
