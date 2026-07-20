//! I/O boundary for [`crate::SegmentBuffer`]: a trait-object store that owns
//! the on-disk representation of segment files.
//!
//! Production code uses [`RealStore`] (real filesystem I/O via `std::fs`).
//! Loom concurrency tests substitute a [`SegmentStore`] implementation
//! backed by `loom::sync::Mutex<HashMap<..>>` so the buffer's
//! `delete_acked` + `append` interleaving can be enumerated exhaustively
//! without modelling the kernel filesystem (loom does not model syscalls).
//!
//! The trait is intentionally minimal: seven methods covering exactly the
//! I/O surface [`crate::SegmentBuffer`] performed inline before this module
//! existed. Each method maps 1:1 to a former `std::fs` call site; the
//! [`RealStore`] implementations are extracted verbatim from the
//! pre-refactor `segment.rs` so on-disk behaviour is byte-identical.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::error::Result;
use crate::segment::{filename, parse_filename, SegmentRange};

/// Suffix for in-progress writes, treated as crash debris on recovery.
/// Mirrors the constant that lived in `segment.rs` before the I/O split.
const TMP_SUFFIX: &str = ".tmp";

/// I/O boundary for [`crate::SegmentBuffer`]: owns the on-disk representation
/// of segment files.
///
/// Production code uses [`RealStore`]. Loom tests substitute a mock
/// implementation backed by `loom::sync::Mutex<HashMap<..>>` so the buffer's
/// mutex-bound invariants can be enumerated exhaustively without modelling
/// the kernel filesystem (loom does not model syscalls).
///
/// # Contract
///
/// Every method receives a [`SegmentRange`] and computes the segment path
/// internally via [`filename`]. Callers never construct paths. The store
/// owns the directory; the buffer does not path-join.
///
/// # Implementation invariants
///
/// - [`SegmentStore::write_atomic`] must be atomic: a concurrent reader must
///   never observe a partial write. [`RealStore`] achieves this via the
///   tmp → `sync_all` → rename sequence; a mock achieves it via a single
///   lock acquisition covering the insert.
/// - [`SegmentStore::remove_segment`] must be idempotent on `NotFound`: a
///   concurrent [`crate::SegmentBuffer::delete_acked`] must not fail when
///   the file was already removed by an earlier delete. Returns `true` when
///   this call actually removed the file, `false` when it was already gone.
/// - [`SegmentStore::segment_size`] returns `0` for a missing segment
///   (mirrors `fs::metadata(..).ok().map(|m| m.len()).unwrap_or(0)`).
///
/// # Errors
///
/// All fallible methods return [`Result<T>`](crate::error::Result).
pub trait SegmentStore: Send + Sync {
    /// Create the segment directory and all parents. Idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] on failure.
    fn create_dir_all(&self) -> Result<()>;

    /// Scan the segment directory and return every segment found, sorted by
    /// `start`. Non-segment files (including `.tmp` debris) are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be read.
    fn scan(&self) -> Result<Vec<SegmentRange>>;

    /// Delete leftover `*.tmp` files from crashed writes. Returns the count
    /// removed. Per-file errors are ignored (best-effort cleanup); only
    /// directory-read errors propagate.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be read.
    fn clean_tmp(&self) -> Result<usize>;

    /// Return the byte length of the segment file for `range`. Returns `0`
    /// when the file is missing or stat'ing fails for any other reason
    /// (mirrors the pre-refactor `fs::metadata(..).unwrap_or(0)` pattern).
    fn segment_size(&self, range: SegmentRange) -> u64;

    /// Remove the segment file for `range`. Returns `true` when this call
    /// removed the file, `false` when it was already gone. Idempotent on
    /// `NotFound` so concurrent
    /// [`delete_acked`](crate::SegmentBuffer::delete_acked) calls do not race
    /// on the same segment.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] on any error other than `NotFound`.
    fn remove_segment(&self, range: SegmentRange) -> Result<bool>;

    /// Atomically write `payload` as the segment file for `range`. Returns
    /// the number of bytes written (the length of `payload`).
    ///
    /// "Atomic" means a concurrent reader never observes a partial write:
    /// either the previous content (or "missing") is visible, or the new
    /// content is — never anything in between.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] on any I/O failure.
    fn write_atomic(&self, range: SegmentRange, payload: &[u8]) -> Result<u64>;

    /// Read the raw bytes of the segment file for `range`.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the file cannot be read.
    fn read_bytes(&self, range: SegmentRange) -> Result<Vec<u8>>;
}

/// Filesystem-backed [`SegmentStore`]. Production default.
///
/// Each method maps 1:1 to a `std::fs` call (or sequence thereof). The
/// implementations are extracted verbatim from the pre-refactor `segment.rs`
/// so on-disk behaviour is byte-identical: the tmp → `sync_all` → rename
/// ordering in [`write_atomic`](SegmentStore::write_atomic), the
/// `read_dir` + `parse_filename` filter in [`scan`](SegmentStore::scan), the
/// idempotent `remove_file` in
/// [`remove_segment`](SegmentStore::remove_segment), etc.
#[derive(Debug)]
pub struct RealStore {
    /// Directory holding `seg_*.zst` files. All segment paths are computed
    /// as `dir.join(filename(range.start, range.end))`.
    dir: PathBuf,
}

impl RealStore {
    /// Construct a [`RealStore`] rooted at `dir`.
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Segment path for `range`. Used internally by every method that needs
    /// to address a specific segment file.
    fn segment_path(&self, range: SegmentRange) -> PathBuf {
        self.dir.join(filename(range.start, range.end))
    }
}

impl SegmentStore for RealStore {
    fn create_dir_all(&self) -> Result<()> {
        Ok(fs::create_dir_all(&self.dir)?)
    }

    fn scan(&self) -> Result<Vec<SegmentRange>> {
        // Verbatim from the pre-refactor `segment::scan`.
        let mut segments = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if let Some(range) = parse_filename(&entry.file_name().to_string_lossy()) {
                segments.push(range);
            }
        }
        segments.sort_by_key(|s| s.start);
        Ok(segments)
    }

    fn clean_tmp(&self) -> Result<usize> {
        // Verbatim from the pre-refactor `segment::clean_tmp`.
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.dir)? {
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

    fn segment_size(&self, range: SegmentRange) -> u64 {
        fs::metadata(self.segment_path(range))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn remove_segment(&self, range: SegmentRange) -> Result<bool> {
        match fs::remove_file(self.segment_path(range)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn write_atomic(&self, range: SegmentRange, payload: &[u8]) -> Result<u64> {
        // Verbatim ordering from the pre-refactor `segment::write`:
        // create tmp → write_all → sync_all → rename. The rename is the
        // atomicity boundary; everything before it must be fsync'd so a
        // crash never leaves a partial segment under the final name.
        let seg_name = filename(range.start, range.end);
        let seg_path = self.dir.join(&seg_name);
        let tmp_path = self.dir.join(format!("{seg_name}{TMP_SUFFIX}"));

        {
            let mut file = fs::File::create(&tmp_path)?;
            file.write_all(payload)?;
            file.sync_all()?;
        }

        fs::rename(&tmp_path, &seg_path)?;
        Ok(payload.len() as u64)
    }

    fn read_bytes(&self, range: SegmentRange) -> Result<Vec<u8>> {
        let path = self.segment_path(range);
        Ok(fs::read(&path)?)
    }
}
