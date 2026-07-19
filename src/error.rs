//! Error types for segment-buffer.

use std::io;

/// Errors produced by segment-buffer operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SegmentError {
    /// Filesystem I/O failure (directory creation, segment read/write, rename, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// CBOR serialization or deserialization failure.
    #[error("CBOR error: {0}")]
    Cbor(String),
    /// Encryption or decryption failure (cipher misconfiguration, key mismatch, etc.).
    #[error("cipher error: {0}")]
    Cipher(String),
    /// Segment file failed an integrity check (truncated, corrupted, nonce missing).
    #[error("segment integrity failure: {0}")]
    Integrity(String),
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SegmentError>;
