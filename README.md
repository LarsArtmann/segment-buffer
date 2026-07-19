# segment-buffer

Durable bounded queue with zstd+CBOR segment files, ack-based deletion, and filename-based crash recovery.

**Extracted from monitor365 (private), proven on 597M+ events.**

## Why?

There are many disk-backed queues in the Rust ecosystem, but none offer this combination:

- **In-memory bounded buffer** that spills to disk on a batch/interval trigger (not always write-through)
- **zstd + CBOR compression** for efficient storage
- **Ack-based deletion** — segments are removed only after the consumer confirms receipt
- **Filename-based crash recovery** — `ls` the directory and you see the state; no WAL, no metadata DB
- **Optional AES-256-GCM encryption at rest** via a pluggable `SegmentCipher` trait
- **MPMC** — multiple writers and readers via `parking_lot::Mutex`

## Install

```bash
cargo add segment-buffer
# optional, for the built-in AES-256-GCM cipher:
cargo add segment-buffer --features encryption
```

## Quickstart

```rust
# use serde::{Deserialize, Serialize};
# #[derive(Serialize, Deserialize, Clone)]
# struct MyItem { id: u64 }
use segment_buffer::{SegmentBuffer, SegmentConfig};

let buffer = SegmentBuffer::<MyItem>::open("/tmp/my-buffer", SegmentConfig::default())?;

// Append items (auto-flushes at the batch threshold or flush interval)
let seq = buffer.append(MyItem { id: 1 })?;

// Read items back (from on-disk segments + in-memory pending)
let items = buffer.read_from(0, 1000)?;

// Delete acknowledged items — a segment is removed when its end_seq <= acked_seq
let last_acked_seq = seq;
let deleted = buffer.delete_acked(last_acked_seq)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Encryption at rest

Enable the `encryption` feature and supply any `SegmentCipher`. The built-in
`AesGcmCipher` writes `[12-byte nonce][ciphertext + GCM tag]` per segment,
byte-compatible with monitor365 so existing encrypted segments read without
migration.

```rust,no_run,no_inline
# use serde::{Deserialize, Serialize};
# #[derive(Serialize, Deserialize, Clone)]
# struct MyItem { id: u64 }
use segment_buffer::{AesGcmCipher, SegmentBuffer, SegmentConfig};

let key = [0u8; 32]; // 32-byte AES-256 key
let cipher = AesGcmCipher::new(&key);
// SegmentConfig is #[non_exhaustive]: Default + field reassignment.
let mut config = SegmentConfig::default();
config.cipher = Some(Box::new(cipher));
let buffer = SegmentBuffer::<MyItem>::open("/tmp/my-buffer", config)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

See `examples/encrypted.rs` for a runnable end-to-end example.

## How it works

```text
append(item) ─► unflushed: Vec<T>   (in-memory, inside the Mutex)
                    │
                    ▼   batch full  OR  flush_interval elapsed  OR  flush()
              take() the batch, compute start_seq/end_seq INSIDE the lock
                    │
                    ▼   (lock released — mutex is never held across file I/O)
              CBOR ─► zstd ─► [optional cipher.encrypt]
                    │
                    ▼
              prepend 8-byte SBF1 envelope ─► write seg_*.zst.tmp ─► fsync ─► atomic rename
                    │
                    ▼   (lock re-acquired)
              approx_disk_bytes += len
```

`read_from(start, limit)` scans on-disk segments (sorted by start) then drains the
in-memory tail. `delete_acked(seq)` removes every segment whose `end <= seq` and
advances `head_seq`. Crash recovery is just: delete `.tmp` debris, parse the
remaining filenames. No WAL, no metadata database.

## Backpressure

The crate ships **metrics, not policy**. `store_pressure()` returns
`approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is `> 0.9`.
You define priority thresholds — see `examples/backpressure.rs`.

## Comparison

| Feature         | segment-buffer           | yaque                     | disk_backed_queue |
| --------------- | ------------------------ | ------------------------- | ----------------- |
| Segment files   | zstd+CBOR                | raw bytes                 | SQLite            |
| Ack/delete      | `delete_acked()`         | `RecvGuard` commit/revert | partial           |
| Crash recovery  | filename-based           | replay or loss            | SQLite WAL        |
| Compression     | zstd                     | none                      | none              |
| In-memory spill | yes (batch threshold)    | no (write-through)        | no                |
| MPMC            | yes (Mutex)              | SPSC only                 | yes               |
| Encryption      | optional (AES-GCM trait) | no                        | no                |

## Status

**v0.4.0** — the "API ergonomic + perf" release. Adds `SegmentConfig::builder()`,
`FlushPolicy` (replaces the silent-combine of batch + interval), `RecoveryReport`,
`for_each_from` lending iterator (~21× faster than `read_from` on in-memory
items), typed `SegmentError::Io { path, source }`, AtomicU64 disk bytes,
`scan_segments()` cache, plus `crash_recovery` and `mpmc` examples.
**v0.4.0 is a breaking release** (SegmentConfig field reshuffle + SegmentError::Io
shape). See [CHANGELOG.md](CHANGELOG.md) for migration notes.
See [FEATURES.md](FEATURES.md), [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md).

**Performance vs v0.1.0:** a controlled `git worktree` benchmark
([docs/perf/2026-07-19_v0.1.0-vs-v0.2.0.md](docs/perf/2026-07-19_v0.1.0-vs-v0.2.0.md))
shows append latency up 30–65% on small batches (the envelope + stats bookkeeping
has a per-write cost) but recovery latency down ~40–45% across the board (the
v0.2.0 recovery refactor). Net is roughly break-even for large-batch workloads
and clearly better on cold starts; tiny-batch high-frequency writers may want
to stay on `=0.1.0` until v0.4.0 hot-path work lands.

## License

Licensed under the [Apache License, Version 2.0](https://github.com/LarsArtmann/segment-buffer/blob/master/LICENSE).
