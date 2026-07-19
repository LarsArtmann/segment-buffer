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

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, info};

use segment::SegmentRange;

/// Configuration knobs for [`SegmentBuffer`].
///
/// This struct is `#[non_exhaustive]`: new fields may be added in any release
/// without breaking semver. Construct via [`SegmentConfig::default()`] and then
/// mutate the public fields you care about:
///
/// ```
/// use segment_buffer::SegmentConfig;
///
/// let mut config = SegmentConfig::default();
/// config.max_batch_events = 64;
/// config.flush_interval_secs = 1;
/// ```
#[non_exhaustive]
pub struct SegmentConfig {
    /// Max events accumulated in RAM before auto-flush (default: 256).
    pub max_batch_events: usize,
    /// Max seconds between flushes. An append after this interval triggers a
    /// flush even if the batch threshold hasn't been reached (default: 5s).
    pub flush_interval_secs: u64,
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
            .field("max_batch_events", &self.max_batch_events)
            .field("flush_interval_secs", &self.flush_interval_secs)
            .field("max_size_bytes", &self.max_size_bytes)
            .field("compression_level", &self.compression_level)
            .field("cipher", &self.cipher.as_ref().map(|_| "[set]"))
            .finish()
    }
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            max_batch_events: 256,
            flush_interval_secs: 5,
            max_size_bytes: 10 * 1024 * 1024 * 1024,
            compression_level: 3,
            cipher: None,
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

struct BufferInner<T> {
    /// Items buffered in memory, not yet written to a segment file. Drained by
    /// [`SegmentBuffer::flush`] and rebuilt empty on crash recovery (unflushed
    /// items do not survive a crash by design).
    unflushed: Vec<T>,
    next_seq: u64,
    head_seq: u64,
    last_flush: Instant,
    approx_disk_bytes: u64,
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
                approx_disk_bytes: 0,
            }),
        };

        buffer.recover()?;
        Ok(buffer)
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
        let (should_flush, seq) = {
            let mut inner = self.inner.lock();
            inner.unflushed.push(event);
            inner.next_seq += 1;
            let seq = inner.next_seq - 1;

            let batch_full = inner.unflushed.len() >= self.config.max_batch_events;
            let interval_elapsed =
                inner.last_flush.elapsed().as_secs() >= self.config.flush_interval_secs;
            (batch_full || interval_elapsed, seq)
        };

        if should_flush {
            self.flush()?;
        }

        Ok(seq)
    }

    /// Flush buffered items to a segment file. No-op if nothing is buffered.
    ///
    /// Flushing is also triggered automatically by [`append`](Self::append)
    /// when the batch threshold (`max_batch_events`) or interval
    /// (`flush_interval_secs`) is reached. Call this explicitly when you need
    /// durability before a known threshold.
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

        {
            let mut inner = self.inner.lock();
            inner.approx_disk_bytes += compressed_len;
        }

        debug!(start_seq, end_seq, count = events.len(), "Flushed segment");
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
        let segments = self.scan_segments()?;
        let mut deleted = 0;
        let mut freed_bytes: u64 = 0;
        let mut new_head = None;

        for seg in &segments {
            if seg.end <= acked_seq {
                let path = self.segment_path(seg.start, seg.end);
                if let Ok(meta) = fs::metadata(&path) {
                    freed_bytes += meta.len();
                }
                match fs::remove_file(&path) {
                    Ok(()) => {
                        deleted += 1;
                        debug!(start = seg.start, end = seg.end, "Deleted acked segment");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            } else if new_head.is_none() {
                new_head = Some(seg.start);
            }
        }

        {
            let mut inner = self.inner.lock();
            inner.approx_disk_bytes = inner.approx_disk_bytes.saturating_sub(freed_bytes);
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
            info!(deleted, freed_bytes, acked_seq, "Deleted acked segments");
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
        let inner = self.inner.lock();
        if self.config.max_size_bytes == 0 {
            return 0.0;
        }
        (inner.approx_disk_bytes as f32 / self.config.max_size_bytes as f32).min(1.0)
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
        let inner = self.inner.lock();
        let pending_count = inner.next_seq.saturating_sub(inner.head_seq);
        let latest_sequence = if inner.next_seq == 0 {
            0
        } else {
            inner.next_seq - 1
        };
        let store_pressure = if self.config.max_size_bytes == 0 {
            0.0
        } else {
            (inner.approx_disk_bytes as f32 / self.config.max_size_bytes as f32).min(1.0)
        };
        BufferStats {
            pending_count,
            latest_sequence,
            head_sequence: inner.head_seq,
            next_sequence: inner.next_seq,
            approx_disk_bytes: inner.approx_disk_bytes,
            max_size_bytes: self.config.max_size_bytes,
            store_pressure,
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn recover(&self) -> Result<()> {
        segment::clean_tmp(&self.dir)?;

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
            inner.approx_disk_bytes = total_bytes;
        }

        info!(
            segments = segment_count,
            head_seq,
            next_seq,
            disk_bytes = total_bytes,
            "Segment buffer recovered"
        );

        Ok(())
    }

    fn write_segment(&self, start: u64, end: u64, events: &[T]) -> Result<u64> {
        segment::write(
            &self.dir,
            self.config.cipher.as_deref(),
            self.config.compression_level,
            SegmentRange::new(start, end),
            events,
        )
    }

    fn read_segment(&self, seg: SegmentRange) -> Result<Vec<T>> {
        segment::read(&self.dir, self.config.cipher.as_deref(), seg)
    }

    fn scan_segments(&self) -> Result<Vec<SegmentRange>> {
        segment::scan(&self.dir)
    }

    fn segment_path(&self, start: u64, end: u64) -> PathBuf {
        self.dir.join(segment::filename(start, end))
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
