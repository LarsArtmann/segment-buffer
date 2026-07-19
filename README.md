# segment-buffer

Durable bounded queue with zstd+CBOR segment files, ack-based deletion, and filename-based crash recovery.

**Extracted from [monitor365](https://github.com/LarsArtmann/monitor365), proven on 597M+ events.**

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
use segment_buffer::{SegmentBuffer, SegmentConfig};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct MyItem { id: u64 }

let buffer = SegmentBuffer::<MyItem>::open("/tmp/my-buffer", SegmentConfig::default())?;

// Append items (auto-flushes at the batch threshold or flush interval)
let seq = buffer.append(MyItem { id: 1 })?;

// Read items back (from on-disk segments + in-memory pending)
let items = buffer.read_from(0, 1000)?;

// Delete acknowledged items — a segment is removed when its end_seq <= acked_seq
let deleted = buffer.delete_acked(last_acked_seq)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Encryption at rest

Enable the `encryption` feature and supply any `SegmentCipher`. The built-in
`AesGcmCipher` writes `[12-byte nonce][ciphertext + GCM tag]` per segment,
byte-compatible with monitor365 so existing encrypted segments read without
migration.

```rust
use segment_buffer::{AesGcmCipher, SegmentBuffer, SegmentConfig};

let cipher = AesGcmCipher::new(&key); // 32-byte AES-256 key
let buffer = SegmentBuffer::<MyItem>::open(
    "/tmp/my-buffer",
    SegmentConfig { cipher: Some(Box::new(cipher)), ..Default::default() },
)?;
```

See `examples/encrypted.rs` for a runnable end-to-end example.

## How it works

```
append(item) ─► unflushed: Vec<T>   (in-memory, inside the Mutex)
                    │
                    ▼   batch full  OR  flush_interval elapsed  OR  flush()
              take() the batch, compute start_seq/end_seq INSIDE the lock
                    │
                    ▼   (lock released — mutex is never held across file I/O)
              CBOR ─► zstd ─► [optional cipher.encrypt]
                    │
                    ▼
              write seg_*.zst.tmp ─► fsync ─► atomic rename to seg_*.zst
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

**v0.2.0** — extracted from monitor365, fully decoupled, zero monitor365
dependencies. The v0.2.0 release hardens the format envelope's legacy-detection
contract (false-positive rate on legacy encrypted files: 2⁻³² → 2⁻⁵⁶), adds
`stats()`/`BufferStats`/`len`/`is_empty`, makes `CipherError` opaque with
`source()` chaining, and ships several correctness refinements. See
[FEATURES.md](FEATURES.md) for the honest capability inventory,
[ROADMAP.md](ROADMAP.md) for direction, and [CHANGELOG.md](CHANGELOG.md) for
the full change history. **v0.2.0 is a breaking release** (error-variant
shapes + `CipherError` field visibility); pin with `=0.1.0` if you need the
older API.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
