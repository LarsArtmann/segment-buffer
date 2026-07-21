# segment-buffer

[![crates.io](https://img.shields.io/crates/v/segment-buffer.svg)](https://crates.io/crates/segment-buffer)
[![docs.rs](https://docs.rs/segment-buffer/badge.svg)](https://docs.rs/segment-buffer)
[![CI](https://github.com/LarsArtmann/segment-buffer/actions/workflows/ci.yml/badge.svg)](https://github.com/LarsArtmann/segment-buffer/actions/workflows/ci.yml)
[![supply chain](https://github.com/LarsArtmann/segment-buffer/actions/workflows/supply-chain-report.yml/badge.svg)](https://github.com/LarsArtmann/segment-buffer/actions/workflows/supply-chain-report.yml)
[![msrv 1.86](https://img.shields.io/static/v1?label=msrv&message=1.86&color=blue)](https://github.com/LarsArtmann/segment-buffer/blob/master/docs/MSRV.md)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

High-throughput **local buffer for cloud sync**. Single-process by design, durability-configurable, optional performant encryption, at-least-once delivery.

**Extracted from monitor365 (private), proven on 597M+ events.**

**Contents:** [Why?](#why) · [Install](#install) · [Quickstart](#quickstart) · [Encryption](#encryption-at-rest) · [Cloud sync drain loop](#cloud-sync-the-at-least-once-drain-loop) · [How it works](#how-it-works) · [Crash behavior](#crash-behavior-configurable) · [Backpressure](#backpressure) · [Comparison](#comparison) · [Status](#status) · [License](#license)

## Why?

The cloud is the durable layer. This crate is the local throughput buffer in front of it. Spool items to disk fast, drain them to your cloud endpoint at your own pace, and delete them only after the server acknowledges. When the cloud is unreachable (offline, partitioned, rate-limited), the buffer holds the backlog; when it comes back, drain resumes from where it left off.

There are many disk-backed queues in the Rust ecosystem, but none target this shape:

- **Single-process by design**: one owner per buffer directory; no cross-process or distributed coordination tax. An exclusive `flock`-based lock at `open()` enforces this (since v0.5.0).
- **Throughput-first, durability-configurable**: pick your crash-resilience level (see [Crash behavior](#crash-behavior-configurable)). When the cloud is the durable copy, skip fsync; when this buffer is the last copy, fsync file + directory.
- **At-least-once delivery built in**: `append()` returns a stable sequence number; `delete_acked(seq)` is the commit point. Crash before the ack and items are re-delivered on recovery. Your server-side handler MUST be idempotent on `(producer_id, seq)`; the library delivers at-least-once, never exactly-once.
- **Optional performant encryption at rest**: AES-256-GCM (byte-compatible with monitor365) and XChaCha20-Poly1305 (extended 24-byte nonce, no 2³²-message limit per key; constant-time in software). `SegmentConfigBuilder::recommended_cipher(key)` installs XChaCha20-Poly1305 for new buffers; legacy AES-GCM segments still decrypt. Streaming/incremental cipher is a long-term direction.
- **zstd + CBOR segment files**: efficient storage, filename-based crash recovery. `ls` the directory and you see the state; no WAL, no metadata DB.

**Use this when** you have a single-process producer that must spool items to disk and drain them to a cloud endpoint at its own pace, with crash recovery and at-least-once delivery.

**Do not use this for** cross-machine replicated queues, multi-process coordination, or server-side fanout. The crate is single-process by design (one `flock` per buffer directory, enforced since v0.5.0); those workloads need a different tool.

## Install

```bash
cargo add segment-buffer
# optional, for the built-in AES-256-GCM and XChaCha20-Poly1305 ciphers:
cargo add segment-buffer --features encryption
```

**MSRV:** 1.86. See [docs/MSRV.md](docs/MSRV.md).

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

// Delete acknowledged items: a segment is removed when its end_seq <= acked_seq
let last_acked_seq = seq;
buffer.delete_acked(last_acked_seq)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Encryption at rest

Enable the `encryption` feature and supply any `SegmentCipher`. The recommended
path for new buffers is `SegmentConfigBuilder::recommended_cipher(key)`, which
installs **XChaCha20-Poly1305** (24-byte extended nonce, no 2³²-message limit
per key, constant-time in software, no AES-NI dependency).

```rust,no_run
# #![allow(unused)]
# // Requires --features encryption. Without it, recommended_cipher does not exist.
# #[cfg(not(feature = "encryption"))]
# fn main() {}
# #[cfg(feature = "encryption")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# use serde::{Deserialize, Serialize};
# #[derive(Serialize, Deserialize, Clone)]
# struct MyItem { id: u64 }
use segment_buffer::{SegmentBuffer, SegmentConfig};

let key = [0u8; 32]; // 32-byte key. In production, load from a KMS / secret store.
// SegmentConfig is #[non_exhaustive]; the builder is the supported construction path.
let buffer = SegmentBuffer::<MyItem>::open(
    "/tmp/my-buffer",
    SegmentConfig::builder().recommended_cipher(key).build(),
)?;
# Ok(())
# }
```

For byte compatibility with monitor365's legacy segment format, install
`AesGcmCipher` explicitly instead; it writes `[12-byte nonce][ciphertext + GCM tag]`
per segment, and legacy AES-GCM segments still decrypt through it. See
`examples/encrypted.rs` and `examples/bring_your_own_cipher.rs` for runnable
end-to-end examples.

### Cloud sync: the at-least-once drain loop

```rust,no_run
# use serde::{Deserialize, Serialize};
# #[derive(Serialize, Deserialize, Clone)]
# struct MyItem { id: u64 }
use segment_buffer::{SegmentBuffer, SegmentConfig};

let buf = SegmentBuffer::<MyItem>::open("/tmp/spool", SegmentConfig::default())?;

// Producer side: append as fast as you can; the buffer handles batching.
for i in 0..10_000 {
    buf.append(MyItem { id: i })?;
}
buf.flush()?; // ensure everything is on disk before draining

// Drain side: read a batch, send to cloud, ack what the server confirmed.
// The starting cursor comes from stats(); from then on, track it yourself.
let mut next = buf.stats().head_sequence;
loop {
    let batch = buf.read_from(next, 1000)?;
    if batch.is_empty() { break; }
    let count = batch.len() as u64;
    cloud_upload(&batch, next)?;          // YOUR idempotent call
    buf.delete_acked(next + count - 1)?;  // commit point
    next += count;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Crash semantics.** If the process dies between `cloud_upload` and `delete_acked`, the batch is still on disk. On restart, `read_from(next, ...)` returns it again; your `cloud_upload` will see the same `(producer, seq)` pairs a second time and must treat them as no-ops. Only the unflushed in-memory tail is at risk of loss; call `flush()` to drain it before crash-sensitive boundaries. The library provides the sequence-number substrate; idempotency lives in your server.

See `examples/cloud_sync.rs` for a full drain loop with retry under transient failures, `examples/cloud_sync_disk_full.rs` for the disk-full backpressure variant, and `examples/idempotent_server.rs` for the matching server-side `(producer_id, seq)` dedup pattern. For p99-sensitive producers, `examples/background_flush.rs` shows the recommended decoupling: `FlushPolicy::Manual` + a caller-owned timer thread.

## How it works

```text
append(item) ─► unflushed: Vec<T>   (in-memory, inside the Mutex)
                    │
                    ▼   batch full  OR  flush_interval elapsed  OR  flush()
              take() the batch, compute start_seq/end_seq INSIDE the lock
                    │
                    ▼   (lock released: mutex is never held across file I/O)
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

## Crash behavior (configurable)

> **Shipped in v0.5.0.** The default remains `Segment` (today's behavior) for one release for backward compatibility; cloud-sync deployments should switch to `Throughput` once the cloud endpoint holds the durable copy. Pick the policy at construction: `SegmentConfig::builder().durability(DurabilityPolicy::Throughput).build()`.

| `DurabilityPolicy`              | Fsync file | Fsync dir after rename | Worst-case crash loss                                         | Use case                                        |
| ------------------------------- | ---------- | ---------------------- | ------------------------------------------------------------- | ----------------------------------------------- |
| `Maximal`                       | yes        | yes                    | last in-flight flush only                                     | standalone queue (this buffer is the last copy) |
| `Segment` _(today's default)_   | yes        | no                     | the rename window (~5–30s of flushes, kernel-dependent)       | backwards-compatible default                    |
| `Throughput` _(for cloud sync)_ | no         | no                     | entire OS dirty window (~30s); the cloud is the durable layer | high-throughput producer with cloud ack         |

Note: today's code fsyncs the segment file's data but not the directory inode after rename, so `Segment` already has a real (small) crash window. `Maximal` closes it at the cost of one extra `dir.sync_all()` per flush. `Throughput` removes the per-flush fsync entirely.

For the full set of performance levers (durability, flush policy, compression level, read path), see [Performance tuning](docs/PERFORMANCE.md#tuning-for-your-workload).

## Backpressure

The crate ships **metrics, not policy**. `store_pressure()` returns
`approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is `> 0.9`.
You define priority thresholds. See `examples/backpressure.rs`. In cloud-sync
deployments the typical policy is: when `store_pressure()` exceeds your
threshold, apply backpressure to the producer (slow down, sample, drop); the
buffer is holding the backlog until the cloud endpoint recovers.

## Comparison

_Comparison tables rot. This one was written against the versions current as of
2026-07; verify against the upstream crates before making a storage decision.
Reframed for the cloud-sync producer-side buffer target._

| Feature         | segment-buffer                          | yaque                     | disk_backed_queue      |
| --------------- | --------------------------------------- | ------------------------- | ---------------------- |
| Target shape    | local spool for cloud sync              | general queue             | general queue          |
| Process model   | single-process (locked at open)         | SPSC                      | multi-process          |
| Segment files   | zstd+CBOR                               | raw bytes                 | SQLite                 |
| Ack/delete      | `delete_acked()` (at-least-once)        | `RecvGuard` commit/revert | partial                |
| Crash recovery  | filename-based                          | replay or loss            | SQLite WAL             |
| Compression     | zstd                                    | none                      | none                   |
| Durability knob | 3 policies (Maximal/Segment/Throughput) | write-through             | SQLite full/normal/off |
| Encryption      | optional (AES-GCM + XChaCha20-Poly1305) | no                        | no                     |

## Status

**Current release (v0.5.1)**: metadata-only patch finishing the v0.5.0 cloud-sync
reframing on the crates.io/docs.rs surfaces. See [CHANGELOG.md](CHANGELOG.md)
for full release history; see [FEATURES.md](FEATURES.md) for the capability
inventory and [ROADMAP.md](ROADMAP.md) for long-term direction and explicit
non-goals.

**Unreleased (master):** performance-only batch (no API or on-disk format
change). See the `[Unreleased]` section of [CHANGELOG.md](CHANGELOG.md) for
details.

**Performance highlight:** the `append/batch_1` benchmark is roughly **2×
faster** than the prior baseline on single-run criterion medians. See
[docs/perf/2026-07-20_hot-path-flamegraph.md](docs/perf/2026-07-20_hot-path-flamegraph.md)
for methodology, and [docs/PERFORMANCE.md](docs/PERFORMANCE.md) for the full
impact-ordered tuning guide. A `Throughput` durability policy removes the
per-flush fsync and opens a further large gain on cloud-sync workloads.

## License

Licensed under the [Apache License, Version 2.0](https://github.com/LarsArtmann/segment-buffer/blob/master/LICENSE).
