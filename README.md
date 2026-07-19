# segment-buffer

Durable bounded queue with zstd+CBOR segment files, ack-based deletion, and crash recovery.

**Extracted from [monitor365](https://github.com/LarsArtmann/monitor365), proven on 597M+ events.**

## Why?

There are many disk-backed queues in the Rust ecosystem, but none offer this combination:

- **In-memory bounded buffer** that spills to disk (not always write-through)
- **zstd + CBOR compression** for efficient storage
- **Ack-based deletion** — segments are removed only after the consumer confirms receipt
- **Filename-based crash recovery** — `ls` the directory and you see the state; no WAL, no metadata DB
- **Optional AES-256-GCM encryption at rest** via a pluggable `SegmentCipher` trait
- **MPMC** — multiple writers and readers via `parking_lot::Mutex`

## Status

**v0.1.0** — extracted from monitor365, fully decoupled, zero monitor365 dependencies.

## Quickstart

```rust
use segment_buffer::{SegmentBuffer, SegmentConfig};

let buffer = SegmentBuffer::<MyItem>::open(
    "/tmp/my-buffer",
    SegmentConfig::default(),
)?;

// Append items (auto-flushes at batch threshold or interval)
let seq = buffer.append(my_item)?;

// Read items back (from disk + in-memory pending)
let items = buffer.read_from(0, 1000)?;

// Delete acknowledged items
let deleted = buffer.delete_acked(last_acked_seq)?;
```

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
| Maintenance     | active                   | stale (Nov 2023)          | minimal           |

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
