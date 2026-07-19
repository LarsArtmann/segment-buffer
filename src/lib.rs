//! Durable bounded queue backed by zstd-compressed CBOR segment files.
//!
//! Items are accumulated in memory, flushed as zstd-compressed CBOR batches
//! to `seg_{start:012}_{end:012}.zst` files, and deleted once the consumer
//! acknowledges receipt via [`SegmentBuffer::delete_acked`].
//!
//! The buffer is generic over any `T: Serialize + DeserializeOwned + Clone + Send + 'static`.
//! Crash recovery is filename-based: scanning the directory rebuilds `head_seq`
//! and `next_seq` without any WAL or metadata database.
//!
//! # Example
//!
//! ```no_run
//! use segment_buffer::{SegmentBuffer, SegmentConfig};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize, Deserialize, Clone)]
//! struct MyItem { id: u64 }
//!
//! let buffer = SegmentBuffer::<MyItem>::open("/tmp/my-queue", SegmentConfig::default())?;
//! let seq = buffer.append(MyItem { id: 1 })?;
//! let items = buffer.read_from(0, 100)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! For the full README — install, quickstart, encryption, backpressure,
//! comparison table, and performance notes — see the
//! [project README on GitHub](https://github.com/LarsArtmann/segment-buffer#segment-buffer)
//! or [docs.rs](https://docs.rs/segment-buffer).

#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

mod cipher;
mod error;
mod segment;

#[cfg(feature = "encryption")]
pub use cipher::AesGcmCipher;
pub use cipher::{CipherError, SegmentCipher};
pub use error::{Result, SegmentError};

/// Hidden internals exposed for fuzz targets and deep integration tests.
/// NOT part of the public API; do not depend on these from external code.
/// Stability is not guaranteed — these may change or disappear in any release.
#[doc(hidden)]
pub mod fuzz_hooks {
    pub use crate::segment::{
        filename, parse_filename, unwrap_envelope, wrap_envelope, SegmentRange,
    };
}

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, info};

use segment::SegmentRange;

/// When to auto-flush pending items from memory to a segment file.
///
/// Passed to [`SegmentConfig`] via its `flush_policy` field. Replaces the
/// pre-v0.4.0 silent combination of two separate fields (`max_batch_events`
/// and `flush_interval_secs`) that OR'd together without telling the caller
/// which trigger fired.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlushPolicy {
    /// Flush as soon as `batch_size` items are buffered. No interval trigger.
    Batch(usize),
    /// Flush as soon as `interval` has elapsed since the last flush. No batch
    /// trigger.
    Interval(std::time::Duration),
    /// Flush when EITHER `batch_size` items are buffered OR `interval` has
    /// elapsed since the last flush — whichever fires first. This is the
    /// pre-v0.4.0 default behavior.
    BatchOrInterval {
        /// In-memory item count threshold.
        batch_size: usize,
        /// Max time between flushes.
        interval: std::time::Duration,
    },
    /// Never auto-flush. The caller must call [`SegmentBuffer::flush`]
    /// explicitly to make appends durable. Useful for tests and for callers
    /// that want absolute control over write amplification.
    Manual,
}

impl Default for FlushPolicy {
    fn default() -> Self {
        // Matches the pre-v0.4.0 SegmentConfig::default: 256 events or 5s.
        FlushPolicy::BatchOrInterval {
            batch_size: 256,
            interval: std::time::Duration::from_secs(5),
        }
    }
}

impl FlushPolicy {
    /// Returns `true` when the policy says the buffer should flush now.
    ///
    /// `pending_len` is the current length of the in-memory `unflushed` Vec;
    /// `time_since_last_flush` is `last_flush.elapsed()`.
    fn should_flush(&self, pending_len: usize, time_since_last_flush: std::time::Duration) -> bool {
        match self {
            FlushPolicy::Batch(n) => pending_len >= *n,
            FlushPolicy::Interval(d) => time_since_last_flush >= *d,
            FlushPolicy::BatchOrInterval {
                batch_size,
                interval,
            } => pending_len >= *batch_size || time_since_last_flush >= *interval,
            FlushPolicy::Manual => false,
        }
    }
}

/// Configuration knobs for [`SegmentBuffer`].
///
/// This struct is `#[non_exhaustive]`: new fields may be added in any release
/// without breaking semver. Construct via [`SegmentConfig::builder()`] and then
/// mutate the public fields you care about, or use [`SegmentConfig::default()`]
/// directly:
///
/// ```
/// use segment_buffer::SegmentConfig;
///
/// let mut config = SegmentConfig::default();
/// config.max_size_bytes = 1024 * 1024;
/// ```
#[non_exhaustive]
pub struct SegmentConfig {
    /// When to auto-flush pending items. See [`FlushPolicy`] for the options.
    pub flush_policy: FlushPolicy,
    /// Max total disk usage before the buffer reports overload pressure (default: 10 GB).
    pub max_size_bytes: u64,
    /// zstd compression level (1-22; 3 is fast with a good ratio).
    pub compression_level: i32,
    /// Optional cipher for encrypting segment files at rest. When `None`,
    /// segments are written as plaintext zstd+CBOR.
    pub cipher: Option<Box<dyn SegmentCipher>>,
}

impl std::fmt::Debug for SegmentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentConfig")
            .field("flush_policy", &self.flush_policy)
            .field("max_size_bytes", &self.max_size_bytes)
            .field("compression_level", &self.compression_level)
            .field("cipher", &self.cipher.as_ref().map(|_| "[set]"))
            .finish()
    }
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            flush_policy: FlushPolicy::default(),
            max_size_bytes: 10 * 1024 * 1024 * 1024,
            compression_level: 3,
            cipher: None,
        }
    }
}

/// Ergonomic builder for [`SegmentConfig`].
///
/// `SegmentConfig` is `#[non_exhaustive]`, so direct struct-literal
/// construction is forbidden outside the crate. The builder is the
/// recommended way for callers to override one or two fields without
/// re-typing every default.
///
/// ```
/// use segment_buffer::{FlushPolicy, SegmentConfig};
/// use std::time::Duration;
///
/// let config = SegmentConfig::builder()
///     .flush_policy(FlushPolicy::Batch(64))
///     .compression_level(6)
///     .build();
/// assert_eq!(config.flush_policy, FlushPolicy::Batch(64));
/// assert_eq!(config.compression_level, 6);
/// // Untouched fields fall back to Default.
/// assert_eq!(config.max_size_bytes, 10 * 1024 * 1024 * 1024);
/// ```
#[derive(Debug)]
pub struct SegmentConfigBuilder {
    inner: SegmentConfig,
}

impl SegmentConfigBuilder {
    /// Override the auto-flush policy. See [`FlushPolicy`] for variants.
    pub fn flush_policy(mut self, policy: FlushPolicy) -> Self {
        self.inner.flush_policy = policy;
        self
    }

    /// Convenience: install a `FlushPolicy::Batch(batch_size)`.
    pub fn flush_at_batch_size(self, batch_size: usize) -> Self {
        self.flush_policy(FlushPolicy::Batch(batch_size))
    }

    /// Convenience: install a `FlushPolicy::Interval(interval)`.
    pub fn flush_at_interval(self, interval: std::time::Duration) -> Self {
        self.flush_policy(FlushPolicy::Interval(interval))
    }

    /// Convenience: install a `FlushPolicy::BatchOrInterval { .. }` with both
    /// triggers set.
    pub fn flush_at_batch_or_interval(
        self,
        batch_size: usize,
        interval: std::time::Duration,
    ) -> Self {
        self.flush_policy(FlushPolicy::BatchOrInterval {
            batch_size,
            interval,
        })
    }

    /// Convenience: install a `FlushPolicy::Manual` (no auto-flush).
    pub fn flush_manually(self) -> Self {
        self.flush_policy(FlushPolicy::Manual)
    }

    /// Override the disk-usage ceiling that triggers `is_overloaded()`.
    pub fn max_size_bytes(mut self, max_size_bytes: u64) -> Self {
        self.inner.max_size_bytes = max_size_bytes;
        self
    }

    /// Override the zstd compression level (1–22; 3 is fast with a good ratio).
    pub fn compression_level(mut self, compression_level: i32) -> Self {
        self.inner.compression_level = compression_level;
        self
    }

    /// Install a [`SegmentCipher`] so segment payloads are encrypted at rest.
    pub fn cipher(mut self, cipher: Box<dyn SegmentCipher>) -> Self {
        self.inner.cipher = Some(cipher);
        self
    }

    /// Materialise the configured [`SegmentConfig`].
    pub fn build(self) -> SegmentConfig {
        self.inner
    }
}

impl SegmentConfig {
    /// Begin a builder. Every field starts at [`SegmentConfig::default`];
    /// chain setter calls to override the ones you care about.
    #[must_use = "the builder is meaningless if discarded"]
    pub fn builder() -> SegmentConfigBuilder {
        SegmentConfigBuilder {
            inner: SegmentConfig::default(),
        }
    }
}

/// Point-in-time snapshot of buffer state, captured atomically under a single
/// lock acquisition so all fields are mutually consistent.
///
/// Returned by [`SegmentBuffer::stats`]. Useful for metrics endpoints or
/// dashboards that need to observe multiple values without paying for several
/// lock/unlock round-trips (and risking a torn read between calls).
///
/// This struct is `#[non_exhaustive]`: new fields may be added in any release
/// without breaking semver. It is constructed internally by [`SegmentBuffer::stats`];
/// callers read fields via dot-syntax or pattern-match with `..` only.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BufferStats {
    /// Items waiting in the buffer (on-disk + in-memory pending).
    /// Same value as [`SegmentBuffer::pending_count`].
    pub pending_count: u64,
    /// Highest sequence number assigned (or `0` if the buffer is empty).
    /// Same value as [`SegmentBuffer::latest_sequence`].
    pub latest_sequence: u64,
    /// Oldest unacknowledged sequence number (`head_seq`).
    pub head_sequence: u64,
    /// Next sequence number that will be assigned by the next successful
    /// [`SegmentBuffer::append`] (`next_seq`).
    pub next_sequence: u64,
    /// Approximate total bytes used by segment files on disk. Decreases when
    /// [`SegmentBuffer::delete_acked`] removes files.
    pub approx_disk_bytes: u64,
    /// Configured ceiling on disk usage (`max_size_bytes`). `0` disables the
    /// limit; in that case [`store_pressure`](Self::store_pressure) is `0.0`.
    pub max_size_bytes: u64,
    /// `approx_disk_bytes / max_size_bytes`, clamped to `[0.0, 1.0]`.
    /// `0.0` when no limit is configured.
    pub store_pressure: f32,
}

/// Summary of the recovery scan performed by [`SegmentBuffer::open`].
///
/// Returned by [`SegmentBuffer::open_with_report`] for programmatic
/// introspection. The same data is logged via `tracing` from
/// [`SegmentBuffer::open`]; this struct is for callers that want to inspect
/// it without parsing logs.
///
/// All fields are snapshots taken during recovery — they may be stale by the
/// time the caller reads them, because other threads can append/flush/delete
/// immediately after `open` returns. For a live view, use
/// [`SegmentBuffer::stats`].
///
/// # Recovering over a populated directory
///
/// ```
/// use segment_buffer::{SegmentBuffer, SegmentConfig, FlushPolicy};
/// use tempfile::tempdir;
///
/// let dir = tempdir()?;
///
/// // First instance: write three items, flush, drop.
/// {
///     let config = SegmentConfig::builder()
///         .flush_policy(FlushPolicy::Manual)
///         .build();
///     let buf: SegmentBuffer<u64> = SegmentBuffer::open(dir.path(), config)?;
///     for i in 0..3u64 { buf.append(i)?; }
///     buf.flush()?;
/// }
///
/// // Re-open: recovery must find one segment covering seqs 0..=2.
/// let (buf, report) =
///     SegmentBuffer::<u64>::open_with_report(dir.path(), SegmentConfig::default())?;
/// assert_eq!(report.segment_count, 1);
/// assert_eq!(report.head_seq, 0);
/// assert_eq!(report.next_seq, 3);
/// assert!(report.disk_bytes > 0, "flushed segment must have nonzero size");
/// assert_eq!(report.removed_tmp_files, 0);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecoveryReport {
    /// Number of valid segment files found on disk during recovery.
    pub segment_count: usize,
    /// Oldest sequence number recovered (the `start` of the first segment),
    /// or `0` when the directory was empty.
    pub head_seq: u64,
    /// Next sequence number that will be assigned by the next
    /// [`SegmentBuffer::append`] (the `end + 1` of the last segment), or `0`
    /// when the directory was empty.
    pub next_seq: u64,
    /// Total bytes of all recovered segment files (sum of file sizes).
    pub disk_bytes: u64,
    /// Number of `.tmp` debris files removed by recovery's cleanup step.
    pub removed_tmp_files: usize,
}

struct BufferInner<T> {
    /// Items buffered in memory, not yet written to a segment file. Drained by
    /// [`SegmentBuffer::flush`] and rebuilt empty on crash recovery (unflushed
    /// items do not survive a crash by design).
    unflushed: Vec<T>,
    next_seq: u64,
    head_seq: u64,
    last_flush: Instant,
}

/// Durable bounded queue of `T` backed by compressed segment files.
///
/// Thread-safe via `parking_lot::Mutex`. All file I/O is synchronous. The mutex
/// is never held across an async boundary because there are no await points.
///
/// Create with [`SegmentBuffer::open`], supplying the directory and config.
pub struct SegmentBuffer<T> {
    dir: PathBuf,
    config: SegmentConfig,
    inner: Mutex<BufferInner<T>>,
    /// Total bytes used by segment files on disk. Updated atomically on
    /// flush/delete/recover so `flush()` does not need to re-acquire the
    /// mutex just to bump one u64. Read by `store_pressure` and `stats`.
    /// Deliberately approximate: the real number can drift if files are
    /// touched outside this crate, so it is suitable for backpressure
    /// signalling and metrics, NOT for billing.
    approx_disk_bytes: std::sync::atomic::AtomicU64,
    /// Cache of `scan_segments()`. `None` means stale (must re-scan); `Some`
    /// means a flush/`delete_acked` has not touched the directory since the
    /// last scan. The cache is invalidated by every on-disk mutation
    /// (`flush`, `delete_acked`, `recover`) and never goes stale any other
    /// way — operators who manipulate the directory behind the buffer's back
    /// get the directory scan cost back.
    scan_cache: Mutex<Option<Vec<segment::SegmentRange>>>,
    /// Re-entrancy guard for [`SegmentBuffer::for_each_from`]. Set to `true`
    /// for the duration of a `for_each_from` call (including across the user
    /// callback `F`); every other `&self` method that takes `inner.lock()`
    /// asserts this is `false` and panics with a clear message otherwise.
    ///
    /// This converts the silent deadlock of re-entering the buffer from inside
    /// a `for_each_from` callback into an immediate, diagnosable panic. The
    /// atomic load costs ~1 ns per locking op — negligible next to the mutex.
    iteration_in_progress: std::sync::atomic::AtomicBool,
}

/// `Debug` mirrors the field set of [`BufferStats`] plus the directory path.
/// It does NOT print the in-memory `unflushed` items (which could be large or
/// sensitive), so `T` itself is not required to be `Debug`.
impl<T> std::fmt::Debug for SegmentBuffer<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("SegmentBuffer")
            .field("dir", &self.dir)
            .field("pending_count", &stats.pending_count)
            .field("latest_sequence", &stats.latest_sequence)
            .field("head_sequence", &stats.head_sequence)
            .field("next_sequence", &stats.next_sequence)
            .field("approx_disk_bytes", &stats.approx_disk_bytes)
            .field("max_size_bytes", &stats.max_size_bytes)
            .field("store_pressure", &stats.store_pressure)
            .finish()
    }
}

impl<T> SegmentBuffer<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    /// Open (or create) a buffer at `dir`, recovering from any existing
    /// segment files.
    ///
    /// Recovery is **filename-based**: it scans the directory to rebuild
    /// `head_seq` / `next_seq` and deletes leftover `.tmp` debris. Segment
    /// *contents* are not read until [`read_from`](Self::read_from), so a
    /// corrupted segment does not fail here — it fails when read.
    ///
    /// If you need the recovery summary (segments found, bytes, head/next seq)
    /// programmatically, use [`SegmentBuffer::open_with_report`] instead. The
    /// same data is logged via `tracing::info!` from this call.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be created or read.
    pub fn open(dir: impl Into<PathBuf>, config: SegmentConfig) -> Result<Self> {
        let (buffer, _report) = Self::open_with_report(dir, config)?;
        Ok(buffer)
    }

    /// Like [`SegmentBuffer::open`], but also returns a [`RecoveryReport`]
    /// describing what the recovery scan found on disk.
    ///
    /// Useful for operational dashboards or migration tools that need to know
    /// the on-disk state without re-scanning.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let (buf, report) =
    ///     SegmentBuffer::<u64>::open_with_report(dir.path(), SegmentConfig::default())?;
    /// assert_eq!(report.segment_count, 0); // fresh dir
    /// assert_eq!(report.head_seq, 0);
    /// assert_eq!(report.next_seq, 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be created or read.
    pub fn open_with_report(
        dir: impl Into<PathBuf>,
        config: SegmentConfig,
    ) -> Result<(Self, RecoveryReport)> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;

        let buffer = Self {
            dir,
            config,
            inner: Mutex::new(BufferInner {
                unflushed: Vec::new(),
                next_seq: 0,
                head_seq: 0,
                last_flush: Instant::now(),
            }),
            approx_disk_bytes: std::sync::atomic::AtomicU64::new(0),
            scan_cache: Mutex::new(None),
            iteration_in_progress: std::sync::atomic::AtomicBool::new(false),
        };

        let report = buffer.recover()?;
        Ok((buffer, report))
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Append an item to the buffer. Assigns the next sequence number and
    /// auto-flushes if the batch threshold or interval is reached.
    ///
    /// Returns the assigned sequence number. The first append returns `0`,
    /// and the number increments by 1 for each subsequent append.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    ///
    /// assert_eq!(buf.append(1)?, 0);
    /// assert_eq!(buf.append(2)?, 1);
    /// assert_eq!(buf.append(3)?, 2);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn append(&self, event: T) -> Result<u64> {
        self.assert_not_reentered("append");
        let (should_flush, seq) = {
            let mut inner = self.inner.lock();
            inner.unflushed.push(event);
            inner.next_seq += 1;
            let seq = inner.next_seq - 1;

            let should_flush = self
                .config
                .flush_policy
                .should_flush(inner.unflushed.len(), inner.last_flush.elapsed());
            (should_flush, seq)
        };

        if should_flush {
            self.flush()?;
        }

        Ok(seq)
    }

    /// Flush buffered items to a segment file. No-op if nothing is buffered.
    ///
    /// Flushing is also triggered automatically by [`append`](Self::append)
    /// according to the configured [`FlushPolicy`] (batch threshold, interval,
    /// both, or manual). Call this explicitly when you need durability before
    /// a known threshold, or when using [`FlushPolicy::Manual`].
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// buf.append(1)?;
    /// buf.append(2)?;
    ///
    /// buf.flush()?; // items now durable on disk
    /// assert_eq!(buf.pending_count(), 2);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn flush(&self) -> Result<()> {
        self.assert_not_reentered("flush");
        let (events, start_seq, end_seq) = {
            let mut inner = self.inner.lock();
            inner.last_flush = Instant::now();
            if inner.unflushed.is_empty() {
                return Ok(());
            }
            let events = std::mem::take(&mut inner.unflushed);
            let count = events.len() as u64;
            let end_seq = inner.next_seq - 1;
            let start_seq = end_seq + 1 - count;
            (events, start_seq, end_seq)
        };

        let compressed_len = self.write_segment(start_seq, end_seq, &events)?;

        // approx_disk_bytes is now an AtomicU64, so flush() no longer needs
        // to re-acquire the mutex just to bump one u64.
        self.approx_disk_bytes
            .fetch_add(compressed_len, std::sync::atomic::Ordering::Relaxed);
        // A new segment file invalidates the directory-scan cache.
        self.invalidate_scan_cache();

        debug!(
            path = self.segment_path(start_seq, end_seq).display().to_string(),
            seq = start_seq,
            end_seq,
            count = events.len(),
            bytes = compressed_len,
            "Flushed segment"
        );
        Ok(())
    }

    /// Read up to `limit` items starting from `start_seq` (inclusive).
    ///
    /// Reads from both on-disk segment files and in-memory pending items.
    /// Items are returned in ascending sequence order.
    ///
    /// Passing `limit = 0` returns an empty `Vec` without scanning.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// buf.append(10)?;
    /// buf.append(20)?;
    /// buf.append(30)?;
    /// buf.flush()?;
    ///
    /// let items = buf.read_from(0, 100)?;
    /// assert_eq!(items, vec![10, 20, 30]);
    ///
    /// // start_seq skips already-read items:
    /// let tail = buf.read_from(2, 100)?;
    /// assert_eq!(tail, vec![30]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn read_from(&self, start_seq: u64, limit: usize) -> Result<Vec<T>> {
        self.assert_not_reentered("read_from");
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut result: Vec<T> = Vec::with_capacity(limit.min(1024));

        // Phase 1: read from on-disk segments.
        let segments = self.scan_segments()?;
        for seg in &segments {
            if result.len() >= limit {
                break;
            }
            if seg.end < start_seq {
                continue;
            }

            let events = self.read_segment(*seg)?;
            let skip = if seg.start < start_seq {
                (start_seq - seg.start) as usize
            } else {
                0
            };

            for event in events.into_iter().skip(skip) {
                if result.len() >= limit {
                    break;
                }
                result.push(event);
            }
        }

        // Phase 2: read from in-memory pending events.
        if result.len() < limit {
            let inner = self.inner.lock();
            let pending_start = inner.next_seq.saturating_sub(inner.unflushed.len() as u64);
            for (i, event) in inner.unflushed.iter().enumerate() {
                let seq = pending_start + i as u64;
                if seq < start_seq {
                    continue;
                }
                if result.len() >= limit {
                    break;
                }
                result.push(event.clone());
            }
        }

        Ok(result)
    }

    /// Lending-iterator counterpart to [`read_from`](Self::read_from): invoke
    /// `f(seq, item)` for up to `limit` items starting at `start_seq`, without
    /// materialising them into a `Vec<T>`.
    ///
    /// This avoids the per-item `Clone` that [`read_from`](Self::read_from)
    /// pays for in-memory pending items. On-disk segments still deserialize
    /// into a temporary `Vec<T>` per segment (the on-disk format is bytes, not
    /// `T`), but items are passed to `f` by reference rather than being
    /// re-collected.
    ///
    /// Returns the number of items the callback was invoked for.
    ///
    /// # Performance
    ///
    /// Micro-benchmarked in `benches/bench_read_vs_for_each.rs` against
    /// in-memory pending items (no segment files):
    ///
    /// | Items | `read_from` | `for_each_from` | Speedup |
    /// |-------|-------------|-----------------|---------|
    /// | 1,000 | ~26 µs      | ~1.2 µs         | ~21×    |
    /// | 10,000| ~200 µs     | ~10 µs          | ~20×    |
    ///
    /// The speedup shrinks toward zero once on-disk segments dominate, because
    /// both paths pay the same CBOR+zstd+cipher decode cost per segment — the
    /// clone saving only applies to the in-memory tail.
    ///
    /// # Re-entrancy contract
    ///
    /// The buffer mutex is held across `f` while iterating the in-memory
    /// pending items. Calling **any** other `&self` method on `SegmentBuffer`
    /// from inside `f` would deadlock (`parking_lot::Mutex` is not reentrant).
    /// To make this footgun impossible to hit silently, every other method
    /// asserts it is not being re-entered from inside a `for_each_from`
    /// callback and **panics with a clear message** if it is. The callback
    /// receives only `(seq, &T)`, which gives no way to reach the buffer, but
    /// a closure that captures a clone of the `Arc<SegmentBuffer<T>>` can
    /// still attempt re-entry — and will now get an immediate, diagnosable
    /// panic instead of a silent hang.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// for i in 0..5u64 {
    ///     buf.append(i * 10)?;
    /// }
    /// buf.flush()?;
    ///
    /// let mut sum = 0u64;
    /// let count = buf.for_each_from(0, 100, |_seq, item| { sum += *item; })?;
    /// assert_eq!(count, 5);
    /// assert_eq!(sum, 0 + 10 + 20 + 30 + 40);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn for_each_from<F>(&self, start_seq: u64, limit: usize, mut f: F) -> Result<usize>
    where
        F: FnMut(u64, &T),
    {
        if limit == 0 {
            return Ok(0);
        }

        // Mark iteration in progress for the entire call. Every other locking
        // method asserts the flag is false and panics with a clear message,
        // converting the silent deadlock (parking_lot::Mutex is not reentrant)
        // into an immediate, diagnosable failure. The guard clears the flag on
        // scope exit, including during panic unwinding from `f`.
        self.iteration_in_progress
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _guard = IterationGuard(&self.iteration_in_progress);

        let mut visited = 0usize;

        // Phase 1: on-disk segments. Items are still deserialized into a per-
        // segment Vec<T>, but each is handed to f by reference rather than
        // being re-collected into the caller's Vec.
        let segments = self.scan_segments()?;
        for seg in &segments {
            if visited >= limit {
                break;
            }
            if seg.end < start_seq {
                continue;
            }

            let events = self.read_segment(*seg)?;
            let skip = if seg.start < start_seq {
                (start_seq - seg.start) as usize
            } else {
                0
            };

            for (offset, event) in events.iter().enumerate().skip(skip) {
                if visited >= limit {
                    break;
                }
                let seq = seg.start + offset as u64;
                f(seq, event);
                visited += 1;
            }
        }

        // Phase 2: in-memory pending items. Here the lending pattern wins:
        // the items are borrowed in place under the lock, with zero clones.
        if visited < limit {
            let inner = self.inner.lock();
            let pending_start = inner.next_seq.saturating_sub(inner.unflushed.len() as u64);
            for (i, event) in inner.unflushed.iter().enumerate() {
                if visited >= limit {
                    break;
                }
                let seq = pending_start + i as u64;
                if seq < start_seq {
                    continue;
                }
                f(seq, event);
                visited += 1;
            }
        }

        Ok(visited)
    }

    /// Delete all on-disk segment files whose items are fully covered by
    /// `acked_seq`.
    ///
    /// A segment is deleted when its `end_seq <= acked_seq`. Returns the number
    /// of segment files removed.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// for i in 0..5u64 {
    ///     buf.append(i)?;
    /// }
    /// buf.flush()?;
    ///
    /// // Consumer has processed sequence 0..=4; acknowledge them:
    /// let removed = buf.delete_acked(4)?;
    /// assert_eq!(removed, 1); // one segment file deleted
    /// assert_eq!(buf.pending_count(), 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Limitation
    ///
    /// Acknowledgement only removes **flushed** segment files. Items still held
    /// in the in-memory pending batch have no segment file to delete, so they
    /// remain readable (and counted by [`SegmentBuffer::pending_count`]) until
    /// they are flushed and acknowledged in a later call. `head_seq` is clamped
    /// so it never advances past the pending window, keeping the backlog count
    /// honest.
    pub fn delete_acked(&self, acked_seq: u64) -> Result<usize> {
        self.assert_not_reentered("delete_acked");
        let segments = self.scan_segments()?;
        let mut deleted = 0;
        let mut freed_bytes: u64 = 0;
        let mut new_head = None;

        for seg in &segments {
            if seg.end <= acked_seq {
                let path = self.segment_path(seg.start, seg.end);
                let file_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                freed_bytes += file_bytes;
                match fs::remove_file(&path) {
                    Ok(()) => {
                        deleted += 1;
                        debug!(
                            path = path.display().to_string(),
                            seq = seg.start,
                            end_seq = seg.end,
                            bytes = file_bytes,
                            "Deleted acked segment"
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            } else if new_head.is_none() {
                new_head = Some(seg.start);
            }
        }

        // Subtract the freed bytes atomically; the lock is still needed for
        // head_seq, but approx_disk_bytes can update independently.
        self.approx_disk_bytes
            .fetch_sub(freed_bytes, std::sync::atomic::Ordering::Relaxed);
        // Deleted segment files invalidate the directory-scan cache.
        self.invalidate_scan_cache();

        {
            let mut inner = self.inner.lock();
            // `head_seq` tracks the oldest unacked sequence. Clamp it to the
            // start of the in-memory pending window: items still waiting to be
            // flushed cannot be acknowledged (there is no segment file to
            // delete), so head_seq must not advance past them. Without this
            // clamp, acknowledging past a buffer that still holds unflushed
            // items would make `pending_count` under-report the real backlog.
            let pending_start = inner.next_seq.saturating_sub(inner.unflushed.len() as u64);
            inner.head_seq = new_head.unwrap_or(inner.next_seq).min(pending_start);
        }

        if deleted > 0 {
            info!(
                path = self.dir.display().to_string(),
                deleted,
                bytes = freed_bytes,
                seq = acked_seq,
                "Deleted acked segments"
            );
        }

        Ok(deleted)
    }

    /// The highest sequence number assigned (or 0 if buffer is empty).
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    ///
    /// assert_eq!(buf.latest_sequence(), 0);
    /// buf.append(7)?;
    /// assert_eq!(buf.latest_sequence(), 0);
    /// buf.append(8)?;
    /// assert_eq!(buf.latest_sequence(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the sequence number is meaningless if discarded"]
    pub fn latest_sequence(&self) -> u64 {
        self.assert_not_reentered("latest_sequence");
        let inner = self.inner.lock();
        if inner.next_seq == 0 {
            0
        } else {
            inner.next_seq - 1
        }
    }

    /// Total items waiting in the buffer (on-disk + in-memory pending).
    ///
    /// Equivalent to `latest_sequence() - head_seq + 1` when non-empty, 0 when
    /// empty. Decreases as [`delete_acked`](Self::delete_acked) removes files.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    ///
    /// assert_eq!(buf.pending_count(), 0);
    /// buf.append(1)?;
    /// buf.append(2)?;
    /// assert_eq!(buf.pending_count(), 2);
    /// buf.flush()?;
    /// assert_eq!(buf.pending_count(), 2); // still pending until acked
    /// buf.delete_acked(1)?;
    /// assert_eq!(buf.pending_count(), 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the backlog size is meaningless if discarded"]
    pub fn pending_count(&self) -> u64 {
        self.assert_not_reentered("pending_count");
        let inner = self.inner.lock();
        inner.next_seq.saturating_sub(inner.head_seq)
    }

    /// Standard [`len`](#method.len) alias for [`pending_count`](Self::pending_count).
    ///
    /// Provided so `SegmentBuffer` reads like a normal collection at the call
    /// site (`buf.len()`, `buf.is_empty()`). Same value as `pending_count()`,
    /// kept as `u64` because the buffer is proven beyond `usize::MAX` on
    /// 32-bit targets (597M+ events in monitor365).
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// assert!(buf.is_empty());
    /// buf.append(7)?;
    /// assert_eq!(buf.len(), 1);
    /// assert!(!buf.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the backlog size is meaningless if discarded"]
    pub fn len(&self) -> u64 {
        self.pending_count()
    }

    /// `true` when there are no items waiting in the buffer (on-disk or
    /// in-memory). Equivalent to `pending_count() == 0`.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// assert!(buf.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the emptiness flag is meaningless if discarded"]
    pub fn is_empty(&self) -> bool {
        self.pending_count() == 0
    }

    /// Disk usage pressure as a value between 0.0 and 1.0.
    ///
    /// Use this to implement your own admission/backpressure policy (e.g.
    /// reject low-priority items above 0.90, reject standard items above 0.95).
    /// Returns 0.0 when `max_size_bytes == 0` (limit disabled).
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let mut cfg = SegmentConfig::default();
    /// cfg.max_size_bytes = 1000; // tiny limit so pressure is observable
    /// let buf: SegmentBuffer<u64> = SegmentBuffer::open(dir.path(), cfg)?;
    ///
    /// assert!(buf.store_pressure() < 0.1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the pressure value is meaningless if discarded"]
    pub fn store_pressure(&self) -> f32 {
        // store_pressure only needs approx_disk_bytes + max_size_bytes —
        // neither requires the mutex. Read the atomic directly to avoid
        // contending with append/flush.
        if self.config.max_size_bytes == 0 {
            return 0.0;
        }
        let bytes = self
            .approx_disk_bytes
            .load(std::sync::atomic::Ordering::Relaxed);
        (bytes as f32 / self.config.max_size_bytes as f32).min(1.0)
    }

    /// True when disk usage exceeds 90% of the configured limit.
    ///
    /// Convenience wrapper around `store_pressure() > 0.9`.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    ///
    /// assert!(!buf.is_overloaded());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the overload flag is meaningless if discarded"]
    pub fn is_overloaded(&self) -> bool {
        self.store_pressure() > 0.9
    }

    /// Capture a consistent snapshot of buffer state under a single lock.
    ///
    /// Cheaper and more consistent than calling
    /// [`pending_count`](Self::pending_count),
    /// [`latest_sequence`](Self::latest_sequence),
    /// [`store_pressure`](Self::store_pressure) etc. individually (which each
    /// take the mutex and could observe a flush/delete between calls).
    ///
    /// # Performance
    ///
    /// Micro-benchmarked in `benches/bench_stats.rs` (run with
    /// `cargo bench --bench bench_stats --features encryption`):
    ///
    /// | Operation                                  | Measured time (median, typical run) |
    /// |--------------------------------------------|--------------------------------------|
    /// | `stats()` (single lock, 7-field snapshot)  | ~12 ns                               |
    /// | 3 individual accessors (`pending_count` + `latest_sequence` + `store_pressure`) | ~31 ns |
    ///
    /// So `stats()` is roughly **2.5× cheaper than 3 individual accessors**
    /// while also being atomic — torn reads between calls are impossible.
    /// Numbers are from the benchmark machine and fluctuate with hardware;
    /// the relative ratio is the durable claim.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// buf.append(1)?;
    /// buf.append(2)?;
    ///
    /// let snapshot = buf.stats();
    /// assert_eq!(snapshot.pending_count, 2);
    /// assert_eq!(snapshot.next_sequence, 2);
    /// assert!(snapshot.store_pressure < 0.01);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the snapshot is meaningless if discarded"]
    pub fn stats(&self) -> BufferStats {
        self.assert_not_reentered("stats");
        let inner = self.inner.lock();
        let pending_count = inner.next_seq.saturating_sub(inner.head_seq);
        let latest_sequence = if inner.next_seq == 0 {
            0
        } else {
            inner.next_seq - 1
        };
        // Load the atomic OUTSIDE the mutex's critical section logic — the
        // value is approximate by design, so a torn read between this load
        // and the inner.lock() is acceptable.
        let approx_disk_bytes = self
            .approx_disk_bytes
            .load(std::sync::atomic::Ordering::Relaxed);
        let store_pressure = if self.config.max_size_bytes == 0 {
            0.0
        } else {
            (approx_disk_bytes as f32 / self.config.max_size_bytes as f32).min(1.0)
        };
        BufferStats {
            pending_count,
            latest_sequence,
            head_sequence: inner.head_seq,
            next_sequence: inner.next_seq,
            approx_disk_bytes,
            max_size_bytes: self.config.max_size_bytes,
            store_pressure,
        }
    }

    /// The directory this buffer reads from and writes segment files to.
    ///
    /// Useful for operators that need to inspect, archive, or quarantine the
    /// segment directory without parsing it out of [`Debug`](std::fmt::Debug).
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// assert_eq!(buf.path(), dir.path());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the path is meaningless if discarded"]
    pub fn path(&self) -> &std::path::Path {
        &self.dir
    }

    /// The [`SegmentConfig`] this buffer was opened with.
    ///
    /// Returned by reference so callers can inspect the flush policy, disk
    /// ceiling, compression level, and cipher presence without re-deriving
    /// them. The config is immutable for the lifetime of the buffer.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig, FlushPolicy};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let config = SegmentConfig::builder()
    ///     .flush_at_batch_size(128)
    ///     .build();
    /// let buf: SegmentBuffer<u64> = SegmentBuffer::open(dir.path(), config)?;
    /// match &buf.config().flush_policy {
    ///     FlushPolicy::Batch(n) => println!("flushing at {n} items"),
    ///     _ => {}
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the config is meaningless if discarded"]
    pub fn config(&self) -> &SegmentConfig {
        &self.config
    }

    /// Re-stat the segment directory and store the authoritative total as
    /// [`BufferStats::approx_disk_bytes`].
    ///
    /// [`BufferStats::approx_disk_bytes`] is updated incrementally on every
    /// flush/delete/recover, so it is accurate as long as only this buffer
    /// touches the directory. If an external process (backup, compaction,
    /// manual cleanup) adds or removes segment files, the cached value drifts.
    /// This method recomputes it from a directory scan.
    ///
    /// Returns the new total so callers can observe the delta without a
    /// second call to [`stats`](Self::stats).
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let buf: SegmentBuffer<u64> =
    ///     SegmentBuffer::open(dir.path(), SegmentConfig::default())?;
    /// buf.append(1)?;
    /// buf.flush()?;
    ///
    /// // Simulate an external process truncating a segment file to zero bytes.
    /// for entry in std::fs::read_dir(dir.path())? {
    ///     let _ = std::fs::write(entry?.path(), b"");
    /// }
    ///
    /// let synced = buf.sync_disk_bytes()?;
    /// assert_eq!(synced, 0, "external truncation should be reflected");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be read.
    pub fn sync_disk_bytes(&self) -> Result<u64> {
        self.assert_not_reentered("sync_disk_bytes");
        let segments = self.scan_segments()?;
        let total: u64 = segments
            .iter()
            .filter_map(|s| fs::metadata(self.segment_path(s.start, s.end)).ok())
            .map(|m| m.len())
            .sum();
        self.approx_disk_bytes
            .store(total, std::sync::atomic::Ordering::Relaxed);
        Ok(total)
    }

    /// Append a batch of items under a single lock acquisition.
    ///
    /// Each item receives the next contiguous sequence number. Returns the
    /// last sequence number assigned (matching the contract of
    /// [`append`](Self::append)); the full range is
    /// `[last - count + 1, last]` where `count` is the number of items the
    /// iterator yielded.
    ///
    /// # Batch vs streaming semantics
    ///
    /// All items are accumulated under a single lock acquisition, then the
    /// flush policy is checked **once** at the end. This gives true atomic
    /// batch semantics: either the entire batch lands in the buffer or the
    /// error propagates. Callers who want per-item auto-flush semantics
    /// (flush at every `batch_size` threshold) should call
    /// [`append`](Self::append) in a loop instead — `append_all` is
    /// optimized for the "load this batch atomically" use case and avoids
    /// paying the lock-acquisition cost per item.
    ///
    /// # Example
    ///
    /// ```
    /// use segment_buffer::{SegmentBuffer, SegmentConfig, FlushPolicy};
    /// use tempfile::tempdir;
    ///
    /// let dir = tempdir()?;
    /// let config = SegmentConfig::builder()
    ///     .flush_policy(FlushPolicy::Manual)
    ///     .build();
    /// let buf: SegmentBuffer<u64> = SegmentBuffer::open(dir.path(), config)?;
    ///
    /// let last = buf.append_all([10u64, 20, 30, 40])?;
    /// assert_eq!(last, 3); // 0-based: items got seqs 0, 1, 2, 3
    /// assert_eq!(buf.pending_count(), 4);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if a flush triggered by the batch fails.
    pub fn append_all<I>(&self, items: I) -> Result<u64>
    where
        I: IntoIterator<Item = T>,
    {
        self.assert_not_reentered("append_all");
        let (should_flush, last_seq, count) = {
            let mut inner = self.inner.lock();
            let mut count = 0u64;
            let mut last_seq = inner.next_seq.saturating_sub(1);
            for item in items {
                inner.unflushed.push(item);
                inner.next_seq = inner.next_seq.wrapping_add(1);
                last_seq = inner.next_seq - 1;
                count += 1;
            }
            if count == 0 {
                // Empty iterator: no-op, return current last seq (or 0).
                return Ok(inner.next_seq.saturating_sub(1));
            }
            let should_flush = self
                .config
                .flush_policy
                .should_flush(inner.unflushed.len(), inner.last_flush.elapsed());
            (should_flush, last_seq, count)
        };
        debug_assert!(count > 0);
        if should_flush {
            self.flush()?;
        }
        Ok(last_seq)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Panic with a clear message if a `for_each_from` callback is currently
    /// re-entering the buffer. The alternative is a silent deadlock
    /// (`parking_lot::Mutex` is not reentrant), so an explicit panic is
    /// strictly better for diagnosability.
    fn assert_not_reentered(&self, method: &'static str) {
        if self
            .iteration_in_progress
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            panic!(
                "{method}: cannot call from within a for_each_from callback \
                 (the buffer mutex is held; re-entry would deadlock)"
            );
        }
    }

    fn recover(&self) -> Result<RecoveryReport> {
        let removed_tmp_files = segment::clean_tmp(&self.dir)?;

        let segments = self.scan_segments()?;

        // All filesystem access (stat'ing each segment for its size) happens
        // BEFORE the mutex is taken. The lock is held only long enough to
        // publish the rebuilt in-memory state, honouring the invariant that
        // the mutex is never held across file I/O.
        let total_bytes: u64 = segments
            .iter()
            .filter_map(|s| fs::metadata(self.segment_path(s.start, s.end)).ok())
            .map(|m| m.len())
            .sum();

        let (head_seq, next_seq) = match (segments.first(), segments.last()) {
            (Some(first), Some(last)) => (first.start, last.end + 1),
            _ => (0, 0),
        };

        let segment_count = segments.len();
        {
            let mut inner = self.inner.lock();
            inner.head_seq = head_seq;
            inner.next_seq = next_seq;
        }
        // Store the recovered disk-bytes total into the atomic directly.
        self.approx_disk_bytes
            .store(total_bytes, std::sync::atomic::Ordering::Relaxed);
        // Recovery just scanned the directory; populate the cache so the
        // first read_from/delete_acked after open does not re-scan.
        *self.scan_cache.lock() = Some(segments.clone());

        info!(
            path = self.dir.display().to_string(),
            segments = segment_count,
            seq = head_seq,
            end_seq = next_seq,
            bytes = total_bytes,
            removed_tmp = removed_tmp_files,
            "Segment buffer recovered"
        );

        Ok(RecoveryReport {
            segment_count,
            head_seq,
            next_seq,
            disk_bytes: total_bytes,
            removed_tmp_files,
        })
    }

    fn write_segment(&self, start: u64, end: u64, events: &[T]) -> Result<u64> {
        let path = self.segment_path(start, end);
        segment::write(
            &self.dir,
            self.config.cipher.as_deref(),
            self.config.compression_level,
            SegmentRange::new(start, end),
            events,
        )
        .map_err(|e| e.with_path(&path))
    }

    fn read_segment(&self, seg: SegmentRange) -> Result<Vec<T>> {
        let path = self.segment_path(seg.start, seg.end);
        segment::read(&self.dir, self.config.cipher.as_deref(), seg).map_err(|e| e.with_path(&path))
    }

    fn scan_segments(&self) -> Result<Vec<SegmentRange>> {
        // Cache hit: clone under the cache lock and return.
        {
            let cache = self.scan_cache.lock();
            if let Some(ref segments) = *cache {
                return Ok(segments.clone());
            }
        }
        // Cache miss: scan the directory, store, return.
        let segments = segment::scan(&self.dir).map_err(|e| e.with_path(&self.dir))?;
        let mut cache = self.scan_cache.lock();
        *cache = Some(segments.clone());
        Ok(segments)
    }

    /// Invalidate the scan cache. Called by every on-disk mutation
    /// (`flush`, `delete_acked`, `recover`).
    fn invalidate_scan_cache(&self) {
        let mut cache = self.scan_cache.lock();
        *cache = None;
    }

    fn segment_path(&self, start: u64, end: u64) -> PathBuf {
        self.dir.join(segment::filename(start, end))
    }
}

/// RAII guard that clears [`SegmentBuffer::iteration_in_progress`] on drop,
/// including during panic unwinding. Without this, a panicking `for_each_from`
/// callback would leave the flag set and permanently brick the buffer.
struct IterationGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for IterationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Static thread-safety assertion
// ---------------------------------------------------------------------------

// `SegmentBuffer<T>` is documented as MPMC-safe via `parking_lot::Mutex`. This
// fails to compile if anyone ever introduces a non-`Send`/`Sync` field on
// `SegmentBuffer` or `BufferInner` (e.g. an `Rc`), turning the documented
// thread-safety guarantee into a compile-time contract instead of a comment.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SegmentBuffer<()>>();
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;

// Each example file is embedded as a doc-test so `cargo test --doc` gives
// execution coverage on top of the compilation coverage from
// `cargo test --examples`. The `concat!` wraps the raw file content in a
// code fence so rustdoc treats it as compilable+runnable Rust.
#[cfg(doctest)]
mod example_doctests {
    #[doc = concat!("```rust\n", include_str!("../examples/basic_usage.rs"), "\n```")]
    const BASIC_USAGE: () = ();

    #[doc = concat!("```rust\n", include_str!("../examples/backpressure.rs"), "\n```")]
    const BACKPRESSURE: () = ();

    #[doc = concat!("```rust\n", include_str!("../examples/crash_recovery.rs"), "\n```")]
    const CRASH_RECOVERY: () = ();

    #[doc = concat!("```rust\n", include_str!("../examples/mpmc.rs"), "\n```")]
    const MPMC: () = ();

    #[cfg(feature = "encryption")]
    #[doc = concat!("```rust\n", include_str!("../examples/encrypted.rs"), "\n```")]
    const ENCRYPTED: () = ();
}
