//! High-throughput **local buffer for cloud sync** — single-process by design,
//! durability-configurable, optional performant encryption, at-least-once delivery.
//!
//! Items are accumulated in memory, flushed as zstd-compressed CBOR batches
//! to `seg_{start:012}_{end:012}.zst` files, and deleted once the consumer
//! acknowledges receipt via [`SegmentBuffer::delete_acked`].
//!
//! The buffer is generic over any `T: Serialize + DeserializeOwned + Clone + Send`.
//! (No explicit `'static` bound is required: `DeserializeOwned` already implies
//! it, since a borrowed type cannot satisfy `for<'de> Deserialize<'de>`.)
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
// Require every public function that can panic or return Result to document
// the failure mode. Prevents the # Panics / # Errors sections from silently
// rotting when new methods land. The 2026-07-20 doc-quality sweep added the
// sections; these lints keep them there.
#![warn(clippy::missing_panics_doc, clippy::missing_errors_doc)]
// Pin the html root URL so intra-doc links resolve against the published
// docs.rs page for this exact version, not whatever rustdoc guessed. Keeps
// `[\`SegmentBuffer\`]`-style links stable across local and docs.rs builds.
// Bump the version segment when cutting a release.
#![doc(html_root_url = "https://docs.rs/segment-buffer/0.5.1")]
// On docs.rs (nightly), enable the `doc_cfg` feature so feature-gated items
// show an "Available on feature `encryption` only" badge. Inert on local
// builds (stable) where `docsrs` is never set.
#![cfg_attr(docsrs, feature(doc_cfg))]
// The crate-root rustdoc is the hand-written block above. The full README
// (install, quickstart, encryption, comparison table, performance) is NOT
// embedded here: it is rendered separately by docs.rs via the `readme` field
// in Cargo.toml, and embedding it via `include_str!` caused two real problems
// — (1) `craneLib.cleanCargoSource` strips README.md from the Nix sandbox,
// needing a `postUnpack` band-aid, and (2) the README's cloud-sync doctest
// referenced an undefined `cloud_upload` fn, turning `cargo test --doc` red.
// Readers reach the README through the links above plus the docs.rs landing
// page; the crate-root stays a concise, self-contained API orientation.

mod cipher;
mod error;
mod segment;
mod store;

#[cfg(feature = "encryption")]
#[cfg_attr(docsrs, doc(cfg(feature = "encryption")))]
pub use cipher::{AesGcmCipher, XChaCha20Poly1305Cipher};
pub use cipher::{CipherError, SegmentCipher};
pub use error::{IoSite, Result, SegmentError};

/// Test/loom-only re-exports: the I/O trait, production impl, and the
/// range type used in trait signatures.
///
/// Reachable only when the `loom` Cargo feature is enabled (used by the
/// `tests/loom.rs` integration test to inject a mock store). Not part of
/// the stable semver surface: items reachable through this re-export may
/// change in any release without a major bump. Mirrors the gating strategy
/// used by `fuzz_hooks`.
#[cfg(feature = "loom")]
pub use segment::SegmentRange;
#[cfg(feature = "loom")]
pub use store::{RealStore, SegmentStore};

/// Internal helpers exposed for in-tree fuzz targets and deep integration tests.
///
/// **Not part of the public API.** Reachable only when the `fuzz` Cargo feature
/// is enabled (or under `cfg(test)`). Stability is not guaranteed — these may
/// change or disappear in any release without bumping the major version.
///
/// Rationale: `#[doc(hidden)]` hides items from rustdoc but does **not** remove
/// them from the semver surface. A `#[cfg]`-gated module does both: it disappears
/// from docs *and* from the compiled crate when the feature is off, so downstream
/// users who never opted into `fuzz` cannot reach these items at all. See
/// `CONTRIBUTING.md` → "Internal hooks: `#[cfg]` over `#[doc(hidden)]`".
#[cfg(any(test, feature = "fuzz"))]
pub mod fuzz_hooks {
    pub use crate::segment::{
        filename, parse_filename, unwrap_envelope, wrap_envelope, SegmentRange,
    };
}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, info};

/// Filename of the single-process lock sidecar held open by every production
/// [`SegmentBuffer`]. Lives inside the segment directory and is acquired
/// exclusively at [`SegmentBuffer::open`]; the kernel releases the lock when
/// the buffer is dropped (closing the fd). Loom-test opens
/// ([`SegmentBuffer::open_with_store`]) skip the lock — loom does not model
/// the filesystem, and a real lock file inside `loom::model` would deadlock.
const LOCK_FILE_NAME: &str = ".segment-buffer.lock";

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

/// Per-flush durability tradeoff between throughput and crash safety.
///
/// Selects how many `fsync`s the write path performs when [`flush`](SegmentBuffer::flush)
/// spills a batch to disk. Higher durability costs throughput; lower
/// durability relies on the cloud (or wherever the durable copy lives) to
/// absorb crash loss. The cloud-sync vision for this crate makes
/// [`Throughput`](Self::Throughput) the natural default once callers opt in,
/// but [`Segment`](Self::Segment) remains the default for one release after
/// the enum lands to avoid silently changing crash semantics for existing
/// users.
///
/// # Crash-loss semantics
///
/// | Policy                 | Fsync file data | Fsync dir after rename | Worst-case crash loss                                |
/// | ---------------------- | --------------- | --------------------- | ---------------------------------------------------- |
/// | [`Maximal`](Self::Maximal)    | yes             | yes                   | last in-flight flush only                            |
/// | [`Segment`](Self::Segment)    | yes             | no                    | rename window (~5–30s of flushes on ext4/xfs)        |
/// | [`Throughput`](Self::Throughput) | no              | no                    | entire OS dirty window (~30s) — cloud is durable     |
///
/// `Maximal` is for standalone-queue deployments where this buffer is the
/// last copy. `Throughput` is the correct choice for cloud-sync deployments
/// where the cloud endpoint holds the durable copy and the local disk is a
/// throughput buffer. `Segment` is the pre-v0.5.0 behavior, kept as the
/// default for one release for backward compatibility.
///
/// # The rename-window gap (why `Segment` is not "fully durable")
///
/// `Segment` (today's default) calls `file.sync_all()` on the segment data
/// before `fs::rename`, but it does **not** `dir.sync_all()` after the
/// rename. On ext4/xfs defaults, a host crash within the kernel's dir-inode
/// flush window (~5–30s) can leave the renamed file's data on disk but
/// unreachable through the directory. SQLite went through this exact lesson.
/// So `Segment` was already not fully durable; the enum just makes the
/// tradeoff explicit. `Maximal` closes the rename-window gap.
///
/// # Implementation
///
/// The policy is branched on inside `SegmentStore::write_atomic`
/// (not a callback): it is a `Copy` enum with no allocation, and the
/// `Mutex<Compressor>` invariant ("never held across I/O") is preserved
/// because the fsync happens after compression is done and the mutex is
/// released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DurabilityPolicy {
    /// Fsync the segment file's data **and** the parent directory inode
    /// after rename. Closes the rename-window gap. Use when this buffer is
    /// the last copy of the data (standalone-queue deployments).
    Maximal,

    /// Fsync the segment file's data, but not the directory inode after
    /// rename. This is the pre-v0.5.0 behavior. Kept as the
    /// [`Default`] for one release after the enum
    /// lands, then flips to [`Throughput`](Self::Throughput) with a
    /// deprecation note.
    #[default]
    Segment,

    /// Skip fsync entirely. The kernel's dirty-page flusher handles when the
    /// bytes reach disk (~30s on default Linux). The rename is still atomic,
    /// so concurrent readers never see a partial write — only a host crash
    /// within the dirty window can lose the segment. Use when the cloud is
    /// the durable layer and this buffer is the throughput buffer in front
    /// of it.
    Throughput,
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
#[derive(Clone)]
pub struct SegmentConfig {
    /// When to auto-flush pending items. See [`FlushPolicy`] for the options.
    pub flush_policy: FlushPolicy,
    /// Max total disk usage before the buffer reports overload pressure (default: 10 GB).
    pub max_size_bytes: u64,
    /// zstd compression level (1-22; default **3**, fast with a good ratio).
    pub compression_level: i32,
    /// Per-flush fsync behavior. See [`DurabilityPolicy`] for the three
    /// policies and their crash-loss tradeoffs. Default is
    /// [`DurabilityPolicy::Segment`] (today's behavior) for backward
    /// compatibility; cloud-sync deployments should switch to
    /// [`DurabilityPolicy::Throughput`] once the cloud endpoint holds the
    /// durable copy.
    pub durability: DurabilityPolicy,
    /// Optional cipher for encrypting segment files at rest. When `None`,
    /// segments are written as plaintext zstd+CBOR. Held as an [`Arc`] so a
    /// [`SegmentConfig`] is [`Clone`] and the same cipher can be shared
    /// across multiple buffers or cloned into a `recommended_cipher()` helper.
    pub cipher: Option<Arc<dyn SegmentCipher + Send + Sync>>,
}

impl std::fmt::Debug for SegmentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentConfig")
            .field("flush_policy", &self.flush_policy)
            .field("max_size_bytes", &self.max_size_bytes)
            .field("compression_level", &self.compression_level)
            .field("durability", &self.durability)
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
            durability: DurabilityPolicy::default(),
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
#[derive(Debug, Clone)]
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

    /// Override the zstd compression level (1-22; default 3, fast with a good ratio).
    pub fn compression_level(mut self, compression_level: i32) -> Self {
        self.inner.compression_level = compression_level;
        self
    }

    /// Override the per-flush durability policy. See [`DurabilityPolicy`] for
    /// the three policies and their crash-loss tradeoffs.
    ///
    /// The default is [`DurabilityPolicy::Segment`] for backward
    /// compatibility. For cloud-sync deployments where the cloud endpoint
    /// holds the durable copy, [`DurabilityPolicy::Throughput`] eliminates
    /// the per-flush fsync from the hot path (typically a 5–10× win on fast
    /// storage).
    pub fn durability(mut self, policy: DurabilityPolicy) -> Self {
        self.inner.durability = policy;
        self
    }

    /// Install a [`SegmentCipher`] so segment payloads are encrypted at rest.
    ///
    /// Accepts an [`Arc`] so the same cipher can be shared across multiple
    /// buffers or cloned into a `recommended_cipher()` helper. The canonical
    /// construction pattern is:
    ///
    /// ```no_run
    /// # #[cfg(feature = "encryption")] {
    /// use segment_buffer::{AesGcmCipher, SegmentConfig};
    /// use std::sync::Arc;
    /// let cfg = SegmentConfig::builder()
    ///     .cipher(Arc::new(AesGcmCipher::new(&[0u8; 32])))
    ///     .build();
    /// # }
    /// ```
    pub fn cipher(mut self, cipher: Arc<dyn SegmentCipher + Send + Sync>) -> Self {
        self.inner.cipher = Some(cipher);
        self
    }

    /// Install the cipher this crate recommends for **new buffers**.
    ///
    /// Available only under the `encryption` feature. Picks
    /// [`XChaCha20Poly1305Cipher`] (24-byte extended nonce, no 2³²-message
    /// limit per key, constant-time on hosts without AES-NI). Legacy
    /// AES-GCM segments still decrypt through [`AesGcmCipher`]; the two
    /// formats are byte-distinguishable only by which cipher the buffer
    /// was opened with.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(feature = "encryption")] {
    /// use segment_buffer::SegmentConfig;
    /// let cfg = SegmentConfig::builder()
    ///     .recommended_cipher([0u8; 32])
    ///     .build();
    /// # }
    /// ```
    #[cfg(feature = "encryption")]
    #[cfg_attr(docsrs, doc(cfg(feature = "encryption")))]
    pub fn recommended_cipher(self, key: [u8; 32]) -> Self {
        self.cipher(Arc::new(XChaCha20Poly1305Cipher::new(&key)))
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

/// High-throughput local buffer for cloud sync, holding items of `T` in
/// memory and spilling them to compressed segment files for at-least-once
/// delivery to a cloud endpoint.
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
    /// Pooled zstd compression context, allocated once at [`SegmentBuffer::open`]
    /// and reused for every subsequent [`SegmentBuffer::flush`]. The flamegraph
    /// captured on 2026-07-20 (see `docs/perf/2026-07-20_hot-path-flamegraph.md`)
    /// showed 66% of `flush` CPU time was inside the `__memset` that
    /// `zstd::encode_all` triggers when it constructs a fresh ~200 KB `CCtx`
    /// per call. Pooling the `CCtx` through `zstd::bulk::Compressor` reduces
    /// that init cost to a one-time `open` expense; subsequent flushes reuse
    /// the same internal tables and pay only the per-frame `SessionOnly` reset
    /// (~0.2% of CPU in the same profile).
    ///
    /// Behind its own `Mutex` (rather than living inside `BufferInner`) so
    /// that holding it during the compression step does not extend the
    /// hot-path `inner` mutex hold time. The mutex is uncontended in
    /// practice: `flush` already takes `inner.lock()` briefly to drain the
    /// pending events, and the re-entrancy guard serialises concurrent
    /// flushers against `for_each_from` anyway.
    compressor: Mutex<zstd::bulk::Compressor<'static>>,
    /// Pooled zstd decompression context — the read-side mirror of
    /// [`compressor`](Self::compressor). Allocated once at
    /// [`SegmentBuffer::open`] and reused for every subsequent
    /// [`SegmentBuffer::read_from`] / [`SegmentBuffer::for_each_from`] call.
    /// Cloud-sync drain loops are read-heavy (draining the buffer is the
    /// primary workload), so the DCtx pooling matters symmetrically to the
    /// CCtx pooling on the write side. Falls back to `zstd::decode_all`
    /// (fresh DCtx per call) only when the frame header lacks a content
    /// size — the bulk::Compressor write path always includes it, so the
    /// fallback is rare in practice (legacy or externally-written files).
    decompressor: Mutex<zstd::bulk::Decompressor<'static>>,
    /// I/O backend. Production uses [`RealStore`] (real filesystem via
    /// `std::fs`); loom concurrency tests inject a mock backed by
    /// `loom::sync::Mutex<HashMap<..>>` so `delete_acked` + `append`
    /// interleavings can be enumerated exhaustively without modelling the
    /// kernel filesystem. The trait object costs ~5 ns per I/O call
    /// (negligible next to zstd+CBOR+file I/O) and is constructed internally
    /// by [`open`](Self::open), so callers never see it. The store is always
    /// called OUTSIDE the `inner` mutex — see [`flush`](Self::flush) and
    /// [`delete_acked`](Self::delete_acked) for the lock-release boundaries.
    store: Arc<dyn store::SegmentStore + Send + Sync>,
    /// File handle holding the exclusive single-process `flock` on
    /// `<dir>/.segment-buffer.lock`. Acquired by `open_internal` BEFORE any
    /// recovery scans or state publication; released by `Drop` (closing the
    /// fd releases the kernel advisory lock). `None` only when the buffer
    /// was constructed via the test-only `open_with_store` path, which
    /// bypasses the lock (loom tests do not model the filesystem and would
    /// otherwise deadlock on a real lock file inside `loom::model`).
    ///
    /// Holding the lock as a `File` rather than via `fs4::FileExt::unlock`
    /// is intentional: the fd-holds-the-lock model is portable (Linux,
    /// macOS, Windows) and survives panics automatically — the kernel
    /// closes the fd on process termination, releasing the lock even if
    /// `Drop` never runs.
    lock_file: Option<std::fs::File>,
    /// Result of the open-time mtime capability probe. `true` when the
    /// filesystem hosting `dir` updates a file's `mtime` on a sub-second
    /// write-after-write window (ext4/xfs/btrfs/apfs/ntfs-defaults all
    /// qualify); `false` when the filesystem pins `mtime` to a constant
    /// (some FUSE mounts, network filesystems with coarse granularity,
    /// memoised-overlay filesystems) — comparing `0 == 0` would falsely
    /// confirm cache validity, so we fall back to today's "cache only
    /// invalidated by in-process mutations" behavior on such filesystems.
    ///
    /// See [`probe_mtime_capability`] for the probe sequence and the
    /// rationale for why a bare stat comparison without the probe is
    /// unsafe.
    mtime_supported: bool,
    /// Last-observed mtime of `dir`, captured alongside every scan_cache
    /// population. Used by [`scan_segments`](Self::scan_segments) to
    /// detect external directory manipulation (a backup tool, a manual
    /// `rm`, an operator quarantining a file) without paying for a full
    /// readdir on every read. Only consulted when [`mtime_supported`](Self::mtime_supported)
    /// is `true`; otherwise the cache stays warm until an in-process
    /// mutation invalidates it.
    last_dir_mtime: Mutex<Option<std::time::SystemTime>>,
}

/// `Debug` mirrors the field set of [`BufferStats`] plus the directory path.
/// It does NOT print the in-memory `unflushed` items (which could be large or
/// sensitive), so `T` itself is not required to be `Debug`.
impl<T> std::fmt::Debug for SegmentBuffer<T>
where
    T: Serialize + DeserializeOwned + Clone + Send,
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
    T: Serialize + DeserializeOwned + Clone + Send,
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
    /// Returns [`SegmentError::Locked`] if another process holds the
    /// exclusive single-process lock on `<dir>/.segment-buffer.lock`.
    pub fn open_with_report(
        dir: impl Into<PathBuf>,
        config: SegmentConfig,
    ) -> Result<(Self, RecoveryReport)> {
        let dir = dir.into();
        let store: Arc<dyn store::SegmentStore + Send + Sync> =
            Arc::new(store::RealStore::new(dir.clone()));
        store.create_dir_all().map_err(|e| e.with_dir())?;

        // Acquire the single-process lock BEFORE any filename parsing or
        // state publication. A second opener on the same directory would
        // race on segment filenames, double-deliver, and corrupt
        // head_seq/next_seq — fail fast with a typed error instead. The
        // lock is held for the lifetime of the returned SegmentBuffer
        // (stored in the `lock_file` field); Drop closes the fd, which
        // releases the kernel advisory lock.
        let lock_path = dir.join(LOCK_FILE_NAME);
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| SegmentError::Io {
                site: IoSite::Segment(lock_path.clone()),
                source,
            })?;
        if fs4::FileExt::try_lock(&lock_file).is_err() {
            return Err(SegmentError::Locked { path: lock_path });
        }
        Self::open_internal(dir, config, store, Some(lock_file))
    }

    /// Open (or create) a buffer with a caller-supplied [`SegmentStore`].
    ///
    /// Production callers use [`open`](Self::open) (which constructs a
    /// [`RealStore`] internally AND acquires the single-process flock).
    /// This constructor exists for loom concurrency tests, which inject a
    /// mock store backed by `loom::sync::Mutex<HashMap<..>>` so
    /// `delete_acked` + `append` interleavings can be enumerated without
    /// modelling the kernel filesystem. It does NOT acquire the flock —
    /// loom does not model the filesystem, and a real lock file inside
    /// `loom::model` would deadlock.
    ///
    /// Only reachable when the `loom` Cargo feature is enabled. Not part of
    /// the stable semver surface.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if `store.create_dir_all()` fails or
    /// recovery cannot scan the segment directory.
    #[cfg(feature = "loom")]
    pub fn open_with_store(
        dir: impl Into<PathBuf>,
        config: SegmentConfig,
        store: Arc<dyn store::SegmentStore + Send + Sync>,
    ) -> Result<Self> {
        let dir = dir.into();
        let (buffer, _report) = Self::open_internal(dir, config, store, None)?;
        Ok(buffer)
    }

    /// Shared constructor used by both the production entry points
    /// (`open`/`open_with_report`) and the test-only `open_with_store`.
    /// Owns the invariant that the store is constructed before recovery
    /// runs, and that `create_dir_all` goes through the store rather than
    /// `std::fs` directly. `lock_file` is `Some` for production opens
    /// (the flock was acquired by the caller) and `None` for loom-test
    /// opens (loom does not model the filesystem).
    fn open_internal(
        dir: PathBuf,
        config: SegmentConfig,
        store: Arc<dyn store::SegmentStore + Send + Sync>,
        lock_file: Option<std::fs::File>,
    ) -> Result<(Self, RecoveryReport)> {
        // `create_dir_all` was already run by the caller if it owned the
        // store (production path). When the test harness passes a fresh
        // store, run it here for symmetry. Idempotent, so a second call is
        // a no-op.
        store.create_dir_all().map_err(|e| e.with_dir())?;

        // Allocate the pooled zstd CCtx once, at the configured compression
        // level. This is the allocation whose per-flush memset was 66% of
        // `flush` CPU before pooling (flamegraph 2026-07-20). The level is
        // fixed for the lifetime of the buffer because `SegmentConfig` is
        // consumed by `open` and immutable thereafter.
        let compressor = zstd::bulk::Compressor::new(config.compression_level)?;
        // Allocate the pooled zstd DCtx once — symmetric to the compressor
        // above. Read paths (`read_from`, `for_each_from`) reuse this DCtx
        // instead of constructing a fresh one per segment decode.
        let decompressor = zstd::bulk::Decompressor::new()?;

        // Probe mtime capability: write a sentinel file twice with a short
        // sleep, and check whether the kernel updated its mtime. On
        // filesystems that pin mtime to a constant (some FUSE, network
        // filesystems with coarse granularity), the scan-cache mtime
        // guard is unsafe (0 == 0 false-positive) and we fall back to
        // today's "cache invalidated only by in-process mutations"
        // behavior. The probe runs at open() time so the cost is paid
        // once. The ~15ms sleep is well within the granularity of every
        // modern local filesystem (ext4/xfs/btrfs/apfs/ntfs all support
        // nanosecond mtime); filesystems that fail the probe are exactly
        // those where the guard would have been unsafe.
        let mtime_supported = probe_mtime_capability(&dir);
        let initial_mtime = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();

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
            compressor: Mutex::new(compressor),
            decompressor: Mutex::new(decompressor),
            store,
            lock_file,
            mtime_supported,
            last_dir_mtime: Mutex::new(initial_mtime),
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns an error only when the auto-flush triggered by this append
    /// fails to write its segment file ([`SegmentError::Io`],
    /// [`SegmentError::Cbor`], or [`SegmentError::Cipher`]). Appends that do
    /// not cross the flush threshold never fail.
    #[track_caller]
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`], [`SegmentError::Cbor`], or
    /// [`SegmentError::Cipher`] if encoding or writing the segment file fails.
    /// A no-op flush (nothing buffered) always succeeds.
    #[track_caller]
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the segment directory cannot be scanned,
    /// or [`SegmentError::Cbor`] / [`SegmentError::Cipher`] /
    /// [`SegmentError::Integrity`] if a segment file cannot be decoded.
    #[track_caller]
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
    /// # Panics
    ///
    /// Panics if called from inside another `for_each_from` callback on the
    /// same buffer (re-entrancy guard converts a silent deadlock into a loud
    /// failure).
    ///
    /// # Errors
    ///
    /// Returns `SegmentError::Io` if any on-disk segment in the requested range
    /// cannot be read or decoded (corruption, missing file after recovery, cipher
    /// failure on an encrypted segment).
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory scan or a segment-file
    /// removal fails.
    #[track_caller]
    pub fn delete_acked(&self, acked_seq: u64) -> Result<usize> {
        self.assert_not_reentered("delete_acked");
        let segments = self.scan_segments()?;
        let mut deleted = 0;
        let mut freed_bytes: u64 = 0;
        let mut new_head = None;

        for seg in &segments {
            if seg.end <= acked_seq {
                let path = self.segment_path(seg.start, seg.end);
                let file_bytes = self.store.segment_size(*seg);
                freed_bytes += file_bytes;
                // remove_segment is idempotent on NotFound so concurrent
                // delete_acked calls do not race on the same segment file.
                // Returns true iff THIS call actually removed the file.
                if self.store.remove_segment(*seg)? {
                    deleted += 1;
                    debug!(
                        path = path.display().to_string(),
                        seq = seg.start,
                        end_seq = seg.end,
                        bytes = file_bytes,
                        "Deleted acked segment"
                    );
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    #[must_use = "the sequence number is meaningless if discarded"]
    #[track_caller]
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    #[must_use = "the backlog size is meaningless if discarded"]
    #[track_caller]
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
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    #[must_use = "the snapshot is meaningless if discarded"]
    #[track_caller]
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
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if the directory cannot be read.
    #[track_caller]
    pub fn sync_disk_bytes(&self) -> Result<u64> {
        self.assert_not_reentered("sync_disk_bytes");
        let segments = self.scan_segments()?;
        let total: u64 = segments.iter().map(|s| self.store.segment_size(*s)).sum();
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
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError::Io`] if a flush triggered by the batch fails.
    #[track_caller]
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

    /// Owned-item iterator over buffer contents starting at `start_seq`.
    ///
    /// Equivalent to [`read_from`](Self::read_from) but yields `(seq, item)`
    /// pairs one at a time so callers can write `for (seq, item) in
    /// buf.iter_from(start, limit)?` and chain standard
    /// [`Iterator`] combinators (`.take`, `.filter`, `.map`, …).
    ///
    /// This is a *materialising* iterator: items are loaded eagerly up to
    /// `limit` (memory cost `O(limit)`). For a *lending* iterator that
    /// passes in-memory items by reference without cloning, use
    /// [`for_each_from`](Self::for_each_from) — that variant is ~20× faster
    /// on the in-memory tail but takes a closure instead of returning an
    /// `Iterator`. The two coexist because no stable-Rust `Iterator`
    /// trait can currently express "yield `&T` from `&mut self`" without
    /// pre-collecting.
    ///
    /// # Re-entrancy contract
    ///
    /// The iterator borrows the buffer for `'a`. Drop the iterator before
    /// calling any other `&self` method on the same buffer; if the
    /// iterator is alive across a `flush` / `append` / `delete_acked`
    /// call, that call will panic with a clear message (same contract as
    /// [`for_each_from`](Self::for_each_from)). The simplest pattern is
    /// `for item in buf.iter_from(..)? { ... }` — the `for` loop drops the
    /// iterator at the end of the block.
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
    /// for i in 0..5u64 { buf.append(i * 10)?; }
    /// buf.flush()?;
    ///
    /// // `for` loop with owned items + seq numbers:
    /// let mut seen = Vec::new();
    /// for (seq, item) in buf.iter_from(0, 100)? {
    ///     seen.push((seq, item));
    /// }
    /// assert_eq!(seen, vec![
    ///     (0, 0), (1, 10), (2, 20), (3, 30), (4, 40),
    /// ]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if called from inside a [`for_each_from`](Self::for_each_from)
    /// callback — the buffer mutex is held across the callback, so re-entry
    /// would deadlock.
    ///
    /// # Errors
    ///
    /// Returns [`SegmentError`] if the directory scan or any segment decode
    /// fails.
    #[track_caller]
    pub fn iter_from(&self, start_seq: u64, limit: usize) -> Result<SegmentIter<'_, T>> {
        self.assert_not_reentered("iter_from");
        if limit == 0 {
            return Ok(SegmentIter {
                inner: Vec::new().into_iter(),
                _phantom: std::marker::PhantomData,
            });
        }
        let items = self.read_from(start_seq, limit)?;
        let indexed: Vec<(u64, T)> = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| (start_seq + i as u64, item))
            .collect();
        Ok(SegmentIter {
            inner: indexed.into_iter(),
            _phantom: std::marker::PhantomData,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Panic with a clear message if a `for_each_from` callback is currently
    /// re-entering the buffer. The alternative is a silent deadlock
    /// (`parking_lot::Mutex` is not reentrant), so an explicit panic is
    /// strictly better for diagnosability.
    #[track_caller]
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
        let removed_tmp_files = self.store.clean_tmp()?;

        let segments = self.scan_segments()?;

        // All store access (sizing each segment) happens BEFORE the mutex is
        // taken. The lock is held only long enough to publish the rebuilt
        // in-memory state, honouring the invariant that the mutex is never
        // held across I/O.
        let total_bytes: u64 = segments.iter().map(|s| self.store.segment_size(*s)).sum();

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
        let range = segment::SegmentRange::new(start, end);
        // Lock the pooled compressor for the duration of the encode. The
        // mutex is uncontended in practice (see field doc) and the lock is
        // NOT held across the store's `write_atomic` call below —
        // `encode_segment` returns bytes before any I/O begins.
        let mut compressor = self.compressor.lock();
        let bytes = segment::encode_segment(
            self.config.cipher.as_deref(),
            &mut compressor,
            &path,
            events,
        )?;
        drop(compressor);
        self.store
            .write_atomic(range, &bytes, self.config.durability)
            .map_err(|e| e.with_path(&path))
    }

    fn read_segment(&self, seg: segment::SegmentRange) -> Result<Vec<T>> {
        let path = self.segment_path(seg.start, seg.end);
        let raw = self.store.read_bytes(seg).map_err(|e| e.with_path(&path))?;
        let mut decompressor = self.decompressor.lock();
        segment::decode_segment(
            self.config.cipher.as_deref(),
            &mut decompressor,
            &raw,
            &path,
        )
        .map_err(|e| e.with_path(&path))
    }

    fn scan_segments(&self) -> Result<Vec<segment::SegmentRange>> {
        // Cache hit: clone under the cache lock and return — UNLESS the
        // directory mtime has moved since the cache was populated (which
        // signals an external mutation: backup tool, manual rm, operator
        // quarantine, etc.). The mtime guard is only consulted when the
        // open-time capability probe confirmed the filesystem actually
        // updates mtime — on filesystems that pin mtime to a constant,
        // comparing 0 == 0 would falsely confirm validity, so we skip the
        // check entirely on those.
        {
            let cache = self.scan_cache.lock();
            if let Some(ref segments) = *cache {
                if !self.mtime_supported || !self.dir_mtime_changed() {
                    return Ok(segments.clone());
                }
                // mtime moved → fall through to re-scan, replacing the cache.
            }
        }
        // Cache miss: scan via the store, store, return.
        let segments = self
            .store
            .scan()
            .map_err(|e| e.with_dir())
            .map_err(|e| e.with_path(&self.dir))?;
        // Refresh the cached dir mtime alongside the cache population so
        // future reads can detect external mutations.
        let fresh_mtime = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        let mut cache = self.scan_cache.lock();
        *cache = Some(segments.clone());
        *self.last_dir_mtime.lock() = fresh_mtime;
        Ok(segments)
    }

    /// Stat the directory's mtime and compare against the last-cached
    /// value. `true` means the directory was touched externally and the
    /// scan cache should be invalidated. Cheap (`stat` is one syscall;
    /// `readdir` is many).
    fn dir_mtime_changed(&self) -> bool {
        let current = match std::fs::metadata(&self.dir).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return true, // directory unreadable → safer to re-scan
        };
        let cached = *self.last_dir_mtime.lock();
        match cached {
            Some(prev) => prev != current,
            None => true,
        }
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

/// Owned-item iterator over buffer contents, yielding `(seq, item)` pairs.
///
/// Returned by [`SegmentBuffer::iter_from`]. Materialises up to `limit`
/// items eagerly (memory cost `O(limit)`); for a lending iterator that
/// passes in-memory items by reference without cloning, use
/// [`SegmentBuffer::for_each_from`].
///
/// The iterator borrows the buffer for `'a`. Drop it before calling any
/// other `&self` method on the same buffer — the re-entrancy contract is
/// the same as [`SegmentBuffer::for_each_from`].
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
/// buf.append(7)?;
/// buf.append(8)?;
/// buf.flush()?;
///
/// let collected: Vec<u64> = buf.iter_from(0, 100)?
///     .map(|(_seq, item)| item)
///     .collect();
/// assert_eq!(collected, vec![7, 8]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct SegmentIter<'a, T> {
    inner: std::vec::IntoIter<(u64, T)>,
    // Tie the iterator's lifetime to the buffer borrow so callers can't
    // outlive the buffer or sneak in a `flush`/`append`/`delete_acked`
    // call while the iterator is live (those methods would panic via
    // assert_not_reentered anyway, but the borrow makes it a compile-time
    // guarantee for &self methods that don't take the inner lock).
    _phantom: std::marker::PhantomData<&'a SegmentBuffer<T>>,
}

impl<T> Iterator for SegmentIter<'_, T> {
    type Item = (u64, T);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T> std::iter::FusedIterator for SegmentIter<'_, T> {}

impl<T> Drop for SegmentBuffer<T> {
    /// Releases the single-process flock by explicitly calling `unlock` and
    /// then dropping the lock file handle. The kernel would release the
    /// advisory lock on fd close anyway, but the explicit call makes the
    /// release point diagnosable in a flamegraph (vs. waiting for `File`'s
    /// own `Drop` to run somewhere in the field-tear-down sequence).
    ///
    /// Deliberately no `T: Serialize + ...` bound: `Drop` impls must match
    /// the struct's bounds (Rust rule E0367), and the struct itself has no
    /// bounds — the bound lives on the API-impl block. The lock-release
    /// logic doesn't touch `T` at all, so no bound is needed here.
    fn drop(&mut self) {
        if let Some(lock_file) = self.lock_file.take() {
            // Best-effort unlock: if it fails (kernel EINTR, already closed,
            // etc.) there is nothing useful to do — the fd is about to be
            // dropped, which releases the lock unconditionally. Suppress the
            // unused-result warning; we already have the strong guarantee.
            let _ = fs4::FileExt::unlock(&lock_file);
            drop(lock_file);
        }
    }
}

/// Probe whether the filesystem at `dir` updates a file's mtime on a
/// sub-second write-after-write window.
///
/// Writes a sentinel file twice with a ~15ms sleep between, then compares
/// the kernel-reported mtime. Modern local filesystems (ext4/xfs/btrfs/
/// apfs/ntfs) all qualify; some FUSE mounts, network filesystems with
/// coarse granularity, and memoised-overlay filesystems pin mtime to a
/// constant and would fail the probe.
///
/// Returns `false` on ANY failure (write error, stat error, mtime
/// unchanged) — the caller treats a `false` as "do not consult mtime when
/// validating the scan cache" (the cache stays warm until an in-process
/// mutation invalidates it). This is the safe default: comparing two
/// `0 == 0` mtimes would falsely confirm cache validity on a no-mtime
/// filesystem, silently serving stale data forever.
fn probe_mtime_capability(dir: &std::path::Path) -> bool {
    let sentinel = dir.join(".segment-buffer.mtime-probe");
    let _ = std::fs::write(&sentinel, b"a");
    let t1 = std::fs::metadata(&sentinel).and_then(|m| m.modified()).ok();
    std::thread::sleep(std::time::Duration::from_millis(15));
    let _ = std::fs::write(&sentinel, b"b");
    let t2 = std::fs::metadata(&sentinel).and_then(|m| m.modified()).ok();
    let _ = std::fs::remove_file(&sentinel);
    matches!((t1, t2), (Some(a), Some(b)) if a != b)
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
