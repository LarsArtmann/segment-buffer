# Domain Language

The ubiquitous vocabulary for `segment-buffer`. Every term here is load-bearing:
it shows up in code identifiers, doc comments, error messages, and commit
messages. Using these terms consistently across code, docs, and issues keeps
the mental model coherent.

> See also: [`CONTRIBUTING.md`](../CONTRIBUTING.md) for the conventions that
> govern how these terms show up in code, and the crate-level rustdoc for the
> API contracts behind each one.

## Core concepts

### Segment

The unit of on-disk storage. A segment is one file named
`seg_{start:012}_{end:012}.zst` containing:

1. An 8-byte `SBF1` envelope (magic + version + reserved bytes).
2. A payload of `zstd(CBOR([T]))`, optionally encrypted via a
   [`SegmentCipher`](#segmentcipher).

A segment is **immutable once written**: `flush()` writes it atomically
(`.tmp` → `fsync` → rename) and it is never modified in place afterwards.
The only mutator that touches a segment file after rename is
[`delete_acked`](#delete_acked), and it removes the whole file.

Segments are the crash-recovery granularity: recovery is `ls` + parse the
filenames. No WAL, no metadata DB.

### Envelope

The 8-byte header prepended to every segment payload:

```text
offset  bytes   meaning
------  -----   -------
  0..4    4     magic: ASCII `SBF1` ("Segment Buffer Format")
  4       1     envelope version (currently 1)
  5..8    3     reserved (all zero; future: checksum type, compression algo…)
  8..           payload (zstd(CBOR([T])), optionally encrypted)
```

Legacy v1 files (pre-v0.2.0, raw encrypted bytes with no magic) are detected
by absence of the `SBF1` magic + zero reserved bytes, and read transparently.
This makes the envelope a strictly additive format change.

### Sequence number (`seq`)

A `u64` assigned to every item at `append` time, starting from 0 on a fresh
buffer. Seqs are **contiguous and monotonically increasing** across the
buffer's lifetime. The seq is the item's identity; the same item value may
appear at multiple seqs, but a seq never refers to more than one item.

Seqs survive crashes: on recovery, the buffer's `next_seq` is set to the
highest end_seq seen across all segment filenames + 1.

### `head_seq`

The oldest **unacknowledged** sequence number still in the buffer.
`head_seq <= next_seq` always. Advanced forward by
[`delete_acked`](#delete_acked). When `head_seq == next_seq`, the buffer is
empty.

### `next_seq`

The seq that will be assigned to the **next** appended item. Starts at 0 on
a fresh buffer (or at the recovery-determined value after `open`).

### `unflushed` (pending)

The in-memory `Vec<T>` of items that have been `append`ed but not yet
written to a segment file. Lives inside the `parking_lot::Mutex<Inner>`.
Flushed to disk by the configured [`FlushPolicy`](#flushpolicy) or by an
explicit `flush()` call.

Items in `unflushed` already have seqs assigned; flushing does not change
their seqs, it only moves them from memory to a segment file.

## Operations

### `append`

`fn append(&self, item: T) -> Result<u64>` — assigns the next seq, pushes
the item into `unflushed`, may trigger a flush per the active
[`FlushPolicy`](#flushpolicy). Returns the assigned seq.

### `append_all`

`fn append_all<I: IntoIterator<Item = T>>(&self, items: I) -> Result<u64>` —
batch append under a single lock acquisition. The whole batch gets
contiguous seqs atomically; flush is checked once at the end. Returns the
last assigned seq (or the current last seq if the iterator was empty).

### `flush`

`fn flush(&self) -> Result<()>` — forcibly drains `unflushed` to a new
segment file, regardless of the [`FlushPolicy`](#flushpolicy) triggers.
Always called from inside the lock-take boundary, but file I/O happens
**outside** the mutex (the mutex is never held across I/O).

### `read_from`

`fn read_from(&self, start: u64, limit: usize) -> Result<Vec<T>>` — returns
up to `limit` items starting at seq `start`, in ascending seq order, from
on-disk segments + in-memory `unflushed`. Items are **cloned** out; this is
the documented cost of the cloning iterator API. See also
[`for_each_from`](#for_each_from) for the zero-copy lending alternative.

### `for_each_from`

`fn for_each_from(&self, start: u64, limit: usize, F: FnMut(u64, &T))` —
lending-iterator variant of [`read_from`](#read_from). Visits the same items
in the same order, but borrows them from the buffer's internal storage
instead of cloning. Holds the mutex across the callback `F`, so re-entering
the buffer from inside `F` panics (the re-entrancy guard converts a silent
deadlock into a loud failure).

### `delete_acked`

`fn delete_acked(&self, acked_seq: u64) -> Result<usize>` — removes every
on-disk segment whose `end_seq <= acked_seq` and advances `head_seq`. Returns
the number of segment files removed. Idempotent: calling with a smaller seq
than a previous call is a no-op.

## Configuration

### `SegmentConfig`

The non-exhaustive config struct. Construct via `SegmentConfig::builder()`
or `SegmentConfig::default()` + field mutation. The active
[`FlushPolicy`](#flushpolicy), compression level, size limit, and optional
cipher live here.

### `FlushPolicy`

When to auto-flush `unflushed` to disk:

- `Batch(n)` — flush as soon as `n` items are buffered.
- `Interval(d)` — flush as soon as `d` has elapsed since the last flush.
- `BatchOrInterval { batch_size, interval }` — flush when **either** fires
  (the pre-v0.4.0 default behavior; still the `Default`).
- `Manual` — never auto-flush; caller must call `flush()` explicitly.

### `SegmentCipher`

Trait abstracting the encrypt/decrypt pair applied to the segment payload
(after envelope strip, before zstd+CBOR decode). Two built-in impls ship
behind the `encryption` feature:

- `AesGcmCipher` — writes `[12-byte nonce][ciphertext + GCM tag]`, byte-
  compatible with the original monitor365 format. Legacy segments still read
  through this cipher.
- `XChaCha20Poly1305Cipher` — writes `[24-byte nonce][ciphertext + Poly1305
tag]`. The 24-byte extended nonce eliminates AES-GCM's 2³²-message-per-key
  limit; constant-time in software (no AES-NI dependency).
  `SegmentConfigBuilder::recommended_cipher(key)` installs this cipher for
  new buffers.

Since the v0.5.0 batch, `SegmentConfig.cipher` is
`Option<Arc<dyn SegmentCipher + Send + Sync>>` (was `Option<Box<…>>`), which
makes `SegmentConfig` and its builder `Clone`. Bring-your-own AEAD is still
supported — any stateless self-describing encrypt/decrypt pair fits the trait.
See [`docs/CIPHERS.md`](./CIPHERS.md).

## Crash recovery

On `open(dir, config)`:

1. Acquire an exclusive `flock` on `<dir>/.segment-buffer.lock` (since v0.5.0;
   returns `SegmentError::Locked { path }` on contention — one owner process
   per directory).
2. Scan `dir` for `*.zst` and `*.tmp` files.
3. Delete `*.tmp` debris (interrupted flush).
4. Parse remaining `seg_*.zst` filenames to recover `(start, end)` ranges.
5. Set `head_seq` to the minimum start across all segments.
6. Set `next_seq` to the maximum end + 1.
7. Sum file sizes into `approx_disk_bytes`.

Recovery is **total and deterministic** — there is no partial state. Either
the buffer opens with the correct seqs, or the directory was corrupt in a
way the API surfaces as a typed error. See `RecoveryReport` for the
post-open summary.

## Storage pressure

`store_pressure() -> f64` returns `approx_disk_bytes / max_size_bytes`,
clamped to `[0.0, 1.0]`. `is_overloaded()` returns `true` when this ratio
exceeds `0.9`. The crate ships **metrics, not policy**: callers decide what
to do with the number (drop, shed load, alert, etc.).

## `DurabilityPolicy` (since v0.5.0)

Selects per-flush fsync behaviour. Three variants:

- `Maximal` — fsync the segment file AND `dir.sync_all()` after rename.
  Closes the rename-window gap. Use when this buffer is the last durable copy.
- `Segment` _(today's default)_ — fsync the segment file only. Already not
  fully durable (the rename window is ~5–30s on ext4/xfs defaults).
- `Throughput` — no fsync; the cloud is the durable layer. Use for cloud-sync
  deployments where the local disk is a throughput buffer.

Threaded through `SegmentStore::write_atomic` as a third parameter.

## `SegmentStore` trait (since v0.5.0)

The I/O boundary of `SegmentBuffer` is an injectable trait object
(`Arc<dyn SegmentStore + Send + Sync>`). The trait covers exactly the former
`std::fs` surface (`create_dir_all`, `scan`, `clean_tmp`, `segment_size`,
`remove_segment`, `write_atomic`, `read_bytes`). Production code constructs a
`RealStore` internally via `open()` / `open_with_report()`; the trait is
reachable externally only under the `loom` Cargo feature
(`SegmentBuffer::open_with_store(dir, config, store)`), and is documented as
not-stable-semver-surface. A loom-aware `MockStore` is how the loom tests
inject an in-memory store.

## `SegmentIter<'_, T>` (since v0.5.0)

Owned-item iterator yielded by `SegmentBuffer::iter_from(start, limit)`.
Returns `(seq, item)` pairs; works with standard `Iterator` combinators
(`.take`, `.filter`, `.map`) and the `for` loop. Materialises up to `limit`
items eagerly. The existing `for_each_from` lending iterator stays for the
zero-copy in-memory tail path.

## `IoSite` (since v0.5.0)

Enum tagging `SegmentError::Io` sites: `Dir`, `Segment(PathBuf)`, or `Unknown`.
Replaces the pre-v0.5.0 `Option<PathBuf>` (where `None` overloaded both
"directory-level failure" and "no context attached yet"). `with_path` and
the new `with_dir` tag Unknown Io errors at high-value call sites.

## `fuzz_hooks`

An opt-in module (`#[cfg(any(test, feature = "fuzz"))]`) exposing internal
helpers — `parse_filename`, `wrap_envelope`, `unwrap_envelope`,
`SegmentRange` — so fuzz targets can drive byte-level invariants directly.
**Not part of the public API.** Items reachable through this feature may
change in any release without a major version bump. See
[`CONTRIBUTING.md` → "Internal hooks: `#[cfg]` over `#[doc(hidden)]`"](../CONTRIBUTING.md).

## Consistency model

The guarantees `SegmentBuffer` provides, organized by what holds under
single-consumer (canonical) vs concurrent (MPMC) operation.

### Canonical usage (single-consumer drain loop)

The intended usage is a single consumer thread running the drain loop:
`read_from → upload → delete_acked`, sequential, no overlap. Under this
pattern the following session guarantees hold:

- **Read-your-writes.** After `append()` returns seq N, a subsequent
  `read_from(N, 1)` sees the item. When `append` does not trigger a flush,
  the item sits in `unflushed` and Phase 2 of `read_from` drains it under the
  lock. When `append` triggers a flush, the flush runs synchronously before
  `append` returns, so the item is already in a segment file on disk and
  Phase 1 of `read_from` scans it.
- **Monotonic reads.** Reading at increasing `start` offsets always shows
  forward progress. Segments are immutable and sequence numbers are
  gap-free and monotonically increasing.
- **Consistent-prefix (contiguous result).** `read_from(start, limit)`
  returns a contiguous run of items from `start`, merging on-disk segments
  with the in-memory tail in ascending seq order. No gaps, no reordering.
- **At-least-once delivery.** Between `read_from` and `delete_acked`, a
  crash leaves the batch on disk. On restart, `read_from` returns it again.

### Concurrent operation (MPPC)

When multiple threads call `read_from`, `flush`, and `delete_acked`
concurrently, two narrow race windows open. Neither causes data loss or
corruption — the items are always correct — but the shape of the result
differs:

- **Spurious Io errors under concurrent `delete_acked`.** `read_from`'s
  Phase 1 scans the directory unlocked; each segment file is then read
  unlocked. If `delete_acked` removes a segment between the scan and the
  file read, `read_from` returns `SegmentError::Io` (`NotFound`). The
  deleted segment was already acknowledged — the error is spurious. Retry
  the read; the next scan reflects the deletion and succeeds.
- **Transient gaps under concurrent `flush`.** `read_from`'s Phase 1 (scan)
  and Phase 2 (read `unflushed` under lock) are separated by an unlocked
  gap. If `flush()` completes during that gap, it moves items from
  `unflushed` into a segment file the scan already missed. `read_from`
  returns an incomplete result for that call. The items are durable on disk
  — a retry sees them.

**What always holds, even under concurrency:**

- Items are never corrupt or reordered. Every item returned by `read_from`
  has the correct seq-to-value mapping. (Proven by
  `concurrent_read_and_delete_never_corrupts` in `src/tests.rs`.)
- `delete_acked` is idempotent and never removes segments whose `end >
acked_seq`. (Proven by loom tests in `tests/loom.rs`.)
- `append` assigns sequence numbers atomically under one lock acquisition.
  (Proven by loom tests.)

**Practical guidance:** the canonical drain loop (one consumer thread,
sequential read → upload → delete) never hits either race. They only
manifest when a second consumer or a background flusher runs concurrently
with the reader. If you need concurrent readers, either (a) serialize
`read_from` and `delete_acked` on the same thread, or (b) retry on
`SegmentError::Io` and accept transient gaps in `read_from` results.

### What is NOT guaranteed

- **Exactly-once delivery.** The crate delivers at-least-once. Making it
  effectively-once requires server-side idempotency on `(producer_id, seq)`.
  See `examples/idempotent_server.rs`.
- **Durability of unflushed items.** Items in `unflushed` are in-memory only.
  A crash loses them. Call `flush()` at crash-sensitive boundaries.
- **Transactional multi-segment reads.** `read_from` is not atomic across
  segments. Each segment is read individually; a failure on segment N
  aborts the read (returns `Err`) without returning segments 0..N-1.
- **Cross-process consistency.** One owner process per directory (enforced
  by `flock`). There is no cross-process coordination.

## Tradeoffs

The crate exposes four knobs for trading durability, latency, memory, CPU,
and disk space against each other. **Consistency and correctness are not
tradeable** — they are invariant by design.

### What you CAN trade

| Knob                                                           | Trades away                | Trades for                                                                    | Mechanism                                                                                                                                                |
| -------------------------------------------------------------- | -------------------------- | ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`DurabilityPolicy`](#durabilitypolicy-since-v050)             | crash resilience           | throughput (fsync calls)                                                      | `Throughput` skips all fsync; `Segment` fsyncs file data only; `Maximal` fsyncs file + directory.                                                        |
| [`FlushPolicy::Batch(n)`](#flushpolicy)                        | p99 latency + peak memory  | throughput (fewer flushes) + disk space (better zstd ratio on larger batches) | Larger `n` → fewer, larger segment files → better compression, fewer syscalls, but bigger write-time spikes and higher memory.                           |
| `compression_level` (1–22)                                     | CPU at flush time          | disk space                                                                    | 1 = fastest, largest files. 22 = slowest, smallest files. Default 3 is a balanced starting point.                                                        |
| [`read_from`](#read_from) vs [`for_each_from`](#for_each_from) | per-item clone allocations | read throughput                                                               | `for_each_from` borrows items from the buffer instead of cloning — ~21× faster on 1k in-memory items. Pays the same decode cost on disk-backed segments. |

### Worked example: maximum throughput, cloud-durable

```rust,ignore
SegmentConfig {
    flush_policy: FlushPolicy::Batch(10_000),  // large batches
    compression_level: 1,                      // minimal CPU
    durability: DurabilityPolicy::Throughput,  // no fsync
    ..SegmentConfig::default()
}
// Drain with for_each_from to avoid cloning.
```

### Worked example: maximum durability, standalone queue

```rust,ignore
SegmentConfig {
    flush_policy: FlushPolicy::Batch(100),     // small batches for low latency
    compression_level: 9,                      // compact on disk
    durability: DurabilityPolicy::Maximal,     // fsync file + directory
    ..SegmentConfig::default()
}
```

### What you CANNOT trade (by design)

| Invariant                     | Why it is fixed                                                                                                                                                                       |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **At-least-once consistency** | Breaking this contradicts the core contract. Items are never silently dropped. The only way to remove items is `delete_acked`, which requires explicit acknowledgment.                |
| **Monotonic reads**           | Follows from segment immutability + monotonic seq assignment. Cannot be traded without fundamentally changing the data model.                                                         |
| **Single-process ownership**  | The `flock`-based lock is the crate's identity. Multi-process would reintroduce split-brain, consensus overhead, and double-delivery — the exact problems this crate exists to avoid. |
| **Synchronous API**           | No hidden threads, no async runtime. An internal flush worker would worsen error propagation (sticky errors on next call vs immediate) and impose a runtime choice on every consumer. |

## Schema evolution of `T`

The crate has two layers of versioning. Understanding the distinction is
critical before changing your item type `T`.

### Envelope versioning (crate-managed)

The 8-byte `SBF1` envelope (magic + version + reserved bytes) is versioned
by the crate. When the crate evolves the format (new compression, checksum,
metadata), it bumps `ENVELOPE_VERSION` in `src/segment.rs`. Legacy files are
auto-detected and read transparently. **You do not control this layer.**

### Payload versioning (caller-managed)

The CBOR payload inside the envelope is `serde`'s serialization of your `T`.
**This layer is completely unversioned.** The crate treats it as opaque
bytes: `serde::serialize` on write, `serde::deserialize` on read. If you
change `T` in a backward-incompatible way, old segment files will fail to
decode. The crate has no way to detect or migrate this — it does not know
the shape of `T`.

This is a deliberate layer boundary: the crate owns the envelope; you own
the payload. (See the layer-split table in `AGENTS.md`.)

### Strategies for evolving `T`

**Compatible changes (no migration needed):**

- **Adding a field with `#[serde(default)]`.** Old segments decode with the
  default value for the new field. This is the safest evolution path.
- **Removing a field.** Old segments carry the field; `serde` silently
  ignores unknown fields during deserialization by default.
- **Renaming via `#[serde(rename = "...")]` or `#[serde(alias = ...)]`.**

**Breaking changes (require a migration plan):**

- Changing a field's type (e.g., `String` → `u64`).
- Making a previously-required field optional without `#[serde(default)]`.
- Changing the enum variant set.

**Mitigation patterns for breaking changes:**

1. **Versioned enum:**

   ```rust,ignore
   #[derive(Serialize, Deserialize)]
   enum MyItem {
       V1(V1Fields),
       V2(V2Fields),
   }
   ```

   Old segments decode as `V1`; new appends write `V2`. The consumer
   matches on the version and applies any per-variant logic.

2. **Upcaster in the drain loop:** read old segments with the old type,
   transform in memory, write to a new buffer. This is a one-time migration
   that runs as a modified drain loop. The crate does not provide upcaster
   infrastructure — see the "no event sourcing framework" non-goal in
   `ROADMAP.md`.

3. **Fresh buffer:** if the backlog is expendable (the cloud already has the
   data), open a new buffer directory with the new `T` and let the old one
   drain to zero before removal.

**When you cannot change `T` at all:** if segments from multiple `T`
versions must coexist in the same directory and the crate must auto-detect
which version to use, you need envelope v2 (cipher id + metadata block).
This is on the `ROADMAP.md` and will not land until a concrete consumer
requires it.
