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

#![warn(missing_docs)]

mod cipher;
mod error;
mod segment;

#[cfg(feature = "encryption")]
pub use cipher::AesGcmCipher;
pub use cipher::SegmentCipher;
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
    /// Returns the assigned sequence number.
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
    pub fn latest_sequence(&self) -> u64 {
        let inner = self.inner.lock();
        if inner.next_seq == 0 {
            0
        } else {
            inner.next_seq - 1
        }
    }

    /// Total items waiting in the buffer (on-disk + in-memory pending).
    pub fn pending_count(&self) -> u64 {
        let inner = self.inner.lock();
        inner.next_seq.saturating_sub(inner.head_seq)
    }

    /// Disk usage pressure as a value between 0.0 and 1.0.
    ///
    /// Use this to implement your own admission/backpressure policy (e.g.
    /// reject low-priority items above 0.90, reject standard items above 0.95).
    pub fn store_pressure(&self) -> f32 {
        let inner = self.inner.lock();
        if self.config.max_size_bytes == 0 {
            return 0.0;
        }
        (inner.approx_disk_bytes as f32 / self.config.max_size_bytes as f32).min(1.0)
    }

    /// True when disk usage exceeds 90% of the configured limit.
    pub fn is_overloaded(&self) -> bool {
        self.store_pressure() > 0.9
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn recover(&self) -> Result<()> {
        segment::clean_tmp(&self.dir)?;

        let segments = self.scan_segments()?;

        let mut inner = self.inner.lock();
        let total_bytes: u64 = segments
            .iter()
            .filter_map(|s| fs::metadata(self.segment_path(s.start, s.end)).ok())
            .map(|m| m.len())
            .sum();

        match (segments.first(), segments.last()) {
            (Some(first), Some(last)) => {
                inner.head_seq = first.start;
                inner.next_seq = last.end + 1;
            }
            _ => {
                inner.next_seq = 0;
                inner.head_seq = 0;
            }
        }
        inner.approx_disk_bytes = total_bytes;

        info!(
            segments = segments.len(),
            head_seq = inner.head_seq,
            next_seq = inner.next_seq,
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
            SegmentRange { start, end },
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

#[cfg(test)]
mod tests;
