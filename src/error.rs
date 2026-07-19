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
//!     Err(SegmentError::Io { path, source }) => {
//!         match path {
//!             Some(p) => eprintln!("I/O failure on {}: {source}", p.display()),
//!             None => eprintln!("unrelated I/O failure: {source}"),
//!         }
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
/// Every variant carries the [`path`](Self::Cbor) of the segment file
/// involved (when one is in scope), so an operator can act on the failure
/// without spelunking through logs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// Filesystem I/O failure (directory creation, segment read/write, rename, etc.).
    ///
    /// Carries the offending `path` when one is in scope, plus the underlying
    /// [`std::io::Error`] as `source`. When `?` propagates an `io::Error`
    /// without context, `path` is `None`; use
    /// [`with_path`](Self::with_path) (or construct the variant directly) to
    /// attach the path at high-value call sites.
    #[error("I/O error{path_clause}: {source}", path_clause = format_path_clause(path))]
    Io {
        /// Path of the file the I/O failed on, when known. `None` for
        /// directory-create or unspecified-path failures.
        path: Option<PathBuf>,
        /// The underlying io::Error, reachable via [`std::error::Error::source`].
        #[source]
        source: std::io::Error,
    },

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

impl SegmentError {
    /// Attach a path to an existing [`SegmentError::Io`] variant. Returns the
    /// error unchanged for other variants. Useful for upgrading a `?`-propagated
    /// io::Error to carry path context at a high-value call site.
    #[must_use = "the upgraded error is meaningless if discarded"]
    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        match self {
            SegmentError::Io { path: _, source } => SegmentError::Io {
                path: Some(path.into()),
                source,
            },
            other => other,
        }
    }
}

impl From<std::io::Error> for SegmentError {
    fn from(source: std::io::Error) -> Self {
        SegmentError::Io { path: None, source }
    }
}

/// Helper used by the `#[error]` attribute on [`SegmentError::Io`]. Produces
/// ` for <path.display()>` when `path` is `Some`, or the empty string when
/// `path` is `None` — so the rendered message has no spurious " for " clause.
fn format_path_clause(path: &Option<PathBuf>) -> String {
    match path {
        Some(p) => format!(" for {}", p.display()),
        None => String::new(),
    }
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SegmentError>;
