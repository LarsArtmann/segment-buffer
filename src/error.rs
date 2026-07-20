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
//! use segment_buffer::{SegmentBuffer, SegmentConfig, SegmentError, IoSite};
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
//!     Err(SegmentError::Io { site, source }) => {
//!         match site {
//!             IoSite::Dir => {
//!                 eprintln!("directory-level I/O failure: {source}");
//!             }
//!             IoSite::Segment(p) => {
//!                 eprintln!("I/O failure on {}: {source}", p.display());
//!             }
//!             IoSite::Unknown => {
//!                 eprintln!("unspecified I/O failure: {source}");
//!             }
//!             // Forward-compat: future sites (e.g. lock file) fall through.
//!             _ => eprintln!("I/O failure: {source}"),
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

/// Which filesystem site an [`SegmentError::Io`] failure happened on.
///
/// Replaces the pre-v0.5.0 `Option<PathBuf>` on the Io variant with an
/// explicit enumeration: directory operations (create_dir_all, scan,
/// clean_tmp, dir fsync), segment-file operations (read/write/rename), and
/// the catch-all for `?`-propagated io::Errors that have not yet been
/// tagged with context.
///
/// `Dir` carries no path: the directory is reachable via
/// [`crate::SegmentBuffer::path`], so the variant just records the *kind*
/// of operation that failed. `Segment` carries the offending segment's
/// path so an operator can quarantine, alert, or move it aside without
/// re-deriving it. `Unknown` is what `?` produces before any high-value
/// call site upgrades the error with [`SegmentError::with_path`] or
/// [`SegmentError::with_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoSite {
    /// The failure happened on the segment directory itself (create_dir_all,
    /// scan, clean_tmp, directory fsync). The directory path is reachable
    /// via [`crate::SegmentBuffer::path`].
    Dir,
    /// The failure happened on a specific segment file. Carries the file's
    /// path so an operator can act on it (move aside, quarantine, alert)
    /// without re-deriving it.
    Segment(PathBuf),
    /// The failure has no specific site attached — typically an io::Error
    /// propagated via `?` before a high-value call site has upgraded it
    /// via [`SegmentError::with_path`] or [`SegmentError::with_dir`].
    Unknown,
}

/// Errors produced by segment-buffer operations.
///
/// Every variant carries the [`site`](Self::Io) or
/// [`path`](Self::Cbor) of the segment file involved (when one is in
/// scope), so an operator can act on the failure without spelunking
/// through logs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// Filesystem I/O failure (directory creation, segment read/write, rename, etc.).
    ///
    /// Carries the [`IoSite`] the failure happened on plus the underlying
    /// [`std::io::Error`] as `source`. When `?` propagates an `io::Error`
    /// without context, the site is [`IoSite::Unknown`]; use
    /// [`with_path`](Self::with_path) (or
    /// [`with_dir`](Self::with_dir)) to attach the site at high-value call
    /// sites.
    #[error("I/O error{site_clause}: {source}", site_clause = format_site_clause(site))]
    Io {
        /// Which site the I/O failed on. See [`IoSite`] for the variants.
        site: IoSite,
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

    /// Another process holds the exclusive single-process lock on the buffer
    /// directory. Returned by [`crate::SegmentBuffer::open`] when the
    /// `flock` on `.segment-buffer.lock` cannot be acquired. See
    /// `AGENTS.md` § "Single-process invariant".
    #[error(
        "buffer directory {path} is locked by another process \
             (only one SegmentBuffer may open a directory at a time)"
    )]
    Locked {
        /// Path of the lock file that was contended (typically
        /// `<dir>/.segment-buffer.lock`).
        path: PathBuf,
    },
}

impl SegmentError {
    /// Attach a segment-file path to an existing [`SegmentError::Io`] variant
    /// whose site is [`IoSite::Unknown`]. Returns the error unchanged for
    /// other variants or for Io errors already tagged (Dir or Segment) —
    /// the first call site to attach context wins.
    ///
    /// Use [`SegmentError::with_dir`] for operations on the directory itself
    /// (create_dir_all, scan, clean_tmp, dir fsync).
    #[must_use = "the upgraded error is meaningless if discarded"]
    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        match self {
            SegmentError::Io {
                site: IoSite::Unknown,
                source,
            } => SegmentError::Io {
                site: IoSite::Segment(path.into()),
                source,
            },
            other => other,
        }
    }

    /// Tag an [`IoSite::Unknown`] Io error as a directory operation. Returns
    /// the error unchanged for other variants or for Io errors already
    /// tagged (Dir or Segment). Use this at directory-operation call sites
    /// (create_dir_all, scan, clean_tmp, dir fsync) so operators can
    /// distinguish "the directory itself failed" from "a specific segment
    /// file failed."
    #[must_use = "the upgraded error is meaningless if discarded"]
    pub fn with_dir(self) -> Self {
        match self {
            SegmentError::Io {
                site: IoSite::Unknown,
                source,
            } => SegmentError::Io {
                site: IoSite::Dir,
                source,
            },
            other => other,
        }
    }
}

impl From<std::io::Error> for SegmentError {
    fn from(source: std::io::Error) -> Self {
        SegmentError::Io {
            site: IoSite::Unknown,
            source,
        }
    }
}

/// Helper used by the `#[error]` attribute on [`SegmentError::Io`]. Produces
/// ` for the segment directory` for [`IoSite::Dir`], ` for {path.display()}`
/// for [`IoSite::Segment`], or the empty string for [`IoSite::Unknown`] — so
/// the rendered message has no spurious clause when no site is attached.
fn format_site_clause(site: &IoSite) -> String {
    match site {
        IoSite::Dir => " for the segment directory".to_string(),
        IoSite::Segment(p) => format!(" for {}", p.display()),
        IoSite::Unknown => String::new(),
    }
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SegmentError>;
