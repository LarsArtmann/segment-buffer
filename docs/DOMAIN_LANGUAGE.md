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
(after envelope strip, before zstd+CBOR decode). The shipped
`AesGcmCipher` (behind the `encryption` feature) writes
`[12-byte nonce][ciphertext + GCM tag]` per segment, byte-compatible with
the original monitor365 format. Bring-your-own AEAD is supported — any
stateless self-describing encrypt/decrypt pair fits the trait. See
[`docs/CIPHERS.md`](./CIPHERS.md).

## Crash recovery

On `open(dir, config)`:

1. Scan `dir` for `*.zst` and `*.tmp` files.
2. Delete `*.tmp` debris (interrupted flush).
3. Parse remaining `seg_*.zst` filenames to recover `(start, end)` ranges.
4. Set `head_seq` to the minimum start across all segments.
5. Set `next_seq` to the maximum end + 1.
6. Sum file sizes into `approx_disk_bytes`.

Recovery is **total and deterministic** — there is no partial state. Either
the buffer opens with the correct seqs, or the directory was corrupt in a
way the API surfaces as a typed error. See `RecoveryReport` for the
post-open summary.

## Storage pressure

`store_pressure() -> f64` returns `approx_disk_bytes / max_size_bytes`,
clamped to `[0.0, 1.0]`. `is_overloaded()` returns `true` when this ratio
exceeds `0.9`. The crate ships **metrics, not policy**: callers decide what
to do with the number (drop, shed load, alert, etc.).

## `fuzz_hooks`

An opt-in module (`#[cfg(any(test, feature = "fuzz"))]`) exposing internal
helpers — `parse_filename`, `wrap_envelope`, `unwrap_envelope`,
`SegmentRange` — so fuzz targets can drive byte-level invariants directly.
**Not part of the public API.** Items reachable through this feature may
change in any release without a major version bump. See
[`CONTRIBUTING.md` → "Internal hooks: `#[cfg]` over `#[doc(hidden)]`"](../CONTRIBUTING.md).
