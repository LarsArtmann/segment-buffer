//! Error types for segment-buffer.
//!
//! Errors carry the context an operator needs to diagnose a failure at 3am:
//! the path to the offending segment file, the phase that failed, and the
//! underlying cause. Use [`Result`](crate::Result) as the alias.
//!
//! # Matching on a failure to recover the offending path
//!
//! Every non-I/O variant carries the segment file's [`PathBuf`](std::path::PathBuf),
//! so an operator can match on the variant and act (move the bad file aside,
//! alert, etc.) without parsing the rendered message:
//!
//! ```
//! use segment_buffer::{SegmentBuffer, SegmentConfig, SegmentError};
//! use tempfile::tempdir;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let dir = tempdir()?;
//! let buf: SegmentBuffer<u64> =
//!     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
//!
//! // Drop a corrupt segment so the next read surfaces a typed error.
//! std::fs::write(
//!     dir.path().join("seg_000000000000_000000000000.zst"),
//!     b"this is not zstd+CBOR",
//! )?;
//!
//! match buf.read_from(0, 10) {
//!     Ok(_) => { /* happy path */ }
//!     Err(SegmentError::Cbor { path, phase, .. }) => {
//!         eprintln!(
//!             "CBOR {phase} failed on {}; quarantining",
//!             path.display()
//!         );
//!         let quarantined = format!("{}.quarantined", path.display());
//!         let _ = std::fs::rename(&path, quarantined);
//!     }
//!     Err(SegmentError::Cipher { path, .. }) => {
//!         eprintln!("cipher failure on {} — likely wrong key", path.display());
//!     }
//!     Err(SegmentError::Integrity { path, reason }) => {
//!         eprintln!("integrity failure on {}: {reason}", path.display());
//!     }
//!     Err(SegmentError::Io(e)) => {
//!         eprintln!("unrelated I/O failure: {e}");
//!     }
//!     // `SegmentError` is `#[non_exhaustive]`, so a catch-all is required
//!     // for forward compatibility with future variants.
//!     Err(other) => {
//!         eprintln!("unhandled segment-buffer error: {other}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::path::PathBuf;

/// Errors produced by segment-buffer operations.
///
/// Every non-I/O variant carries the [`path`](Self::Cbor) of the segment file
/// involved, so an operator can act on the failure without spelunking through
/// logs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// Filesystem I/O failure (directory creation, segment read/write, rename, etc.).
    ///
    /// Kept as a tuple variant with `#[from]` so `?` remains ergonomic at the
    /// many I/O call sites where per-site context would add noise without value.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// CBOR serialization or deserialization of a segment file failed.
    #[error("CBOR {phase} failed for {path}: {message}")]
    Cbor {
        /// Which direction failed: `"serialize"` (writing) or `"deserialize"` (reading).
        phase: &'static str,
        /// Path to the offending segment file.
        path: PathBuf,
        /// Underlying CBOR error message.
        message: String,
    },

    /// Cipher encrypt or decrypt of a segment file failed (key mismatch, AEAD
    /// tag invalid, cipher misconfiguration).
    #[error("cipher error for {path}: {message}")]
    Cipher {
        /// Path to the offending segment file.
        path: PathBuf,
        /// Underlying cipher error message.
        message: String,
    },

    /// Segment file failed an integrity check: truncated, too small for the
    /// AEAD nonce, or unrecognized envelope.
    #[error("integrity failure for {path}: {reason}")]
    Integrity {
        /// Path to the offending segment file.
        path: PathBuf,
        /// What failed, in one short phrase.
        reason: &'static str,
    },
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SegmentError>;
