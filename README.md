# segment-buffer

High-throughput **local buffer for cloud sync**. Single-process by design, durability-configurable, optional performant encryption, at-least-once delivery.

**Extracted from monitor365 (private), proven on 597M+ events.**

## Why?

The cloud is the durable layer. This crate is the local throughput buffer in front of it — spool items to disk fast, drain them to your cloud endpoint at your own pace, and delete them only after the server acknowledges. When the cloud is unreachable (offline, partitioned, rate-limited), the buffer holds the backlog; when it comes back, drain resumes from where it left off.

There are many disk-backed queues in the Rust ecosystem, but none target this shape:

- **Single-process by design** — one owner per buffer directory; no cross-process or distributed coordination tax. A `flock`-based exclusive lock at `open()` will make this enforceable, not just documented (planned for v0.5.0).
- **Throughput-first, durability-configurable** — pick your crash-resilience level (see [Crash behavior](#crash-behavior-configurable)). When the cloud is the durable copy, skip fsync; when this buffer is the last copy, fsync file + directory.
- **At-least-once delivery built in** — `append()` returns a stable sequence number; `delete_acked(seq)` is the commit point. Crash before the ack and items are re-delivered on recovery. Your server-side handler MUST be idempotent on `(producer_id, seq)`; the library delivers at-least-once, never exactly-once.
- **Optional performant encryption at rest** — AES-256-GCM today (byte-compatible with monitor365); XChaCha20-Poly1305 (extended nonce, no 2³²-message limit per key) is the planned default for new buffers. Streaming/incremental cipher is a long-term direction.
- **zstd + CBOR segment files** — efficient storage, filename-based crash recovery. `ls` the directory and you see the state; no WAL, no metadata DB.

For cross-machine replicated queues or server-side fanout, use a different tool — this crate is the producer-side local buffer.

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

```rust,no_run
# #![allow(unused)]
# // Requires --features encryption. Without it, AesGcmCipher does not exist.
# #[cfg(not(feature = "encryption"))]
# fn main() {}
# #[cfg(feature = "encryption")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
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
# Ok(())
# }
```

See `examples/encrypted.rs` for a runnable end-to-end example.

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

**Crash semantics.** If the process dies between `cloud_upload` and `delete_acked`, the batch is still on disk. On restart, `read_from(next, ...)` returns it again — your `cloud_upload` will see the same `(producer, seq)` pairs a second time and must treat them as no-ops. Only the unflushed in-memory tail is at risk of loss; call `flush()` to drain it before crash-sensitive boundaries. The library provides the sequence-number substrate; idempotency lives in your server.

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

## Crash behavior (configurable)

> **Shipped in v0.5.0.** The default remains `Segment` (today's behavior) for one release for backward compatibility; cloud-sync deployments should switch to `Throughput` once the cloud endpoint holds the durable copy. Pick the policy at construction: `SegmentConfig::builder().durability(DurabilityPolicy::Throughput).build()`.

| `DurabilityPolicy`              | Fsync file | Fsync dir after rename | Worst-case crash loss                                         | Use case                                        |
| ------------------------------- | ---------- | ---------------------- | ------------------------------------------------------------- | ----------------------------------------------- |
| `Maximal`                       | yes        | yes                    | last in-flight flush only                                     | standalone queue — this buffer is the last copy |
| `Segment` _(today's default)_   | yes        | no                     | the rename window (~5–30s of flushes, kernel-dependent)       | backwards-compatible default                    |
| `Throughput` _(for cloud sync)_ | no         | no                     | entire OS dirty window (~30s); the cloud is the durable layer | high-throughput producer with cloud ack         |

Note: today's code fsyncs the segment file's data but not the directory inode after rename, so `Segment` already has a real (small) crash window. `Maximal` closes it at the cost of one extra `dir.sync_all()` per flush. `Throughput` removes the per-flush fsync entirely.

## Backpressure

The crate ships **metrics, not policy**. `store_pressure()` returns
`approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is `> 0.9`.
You define priority thresholds — see `examples/backpressure.rs`. In cloud-sync
deployments the typical policy is: when `store_pressure()` exceeds your
threshold, apply backpressure to the producer (slow down, sample, drop) — the
buffer is holding the backlog until the cloud endpoint recovers.

## Comparison

_Comparison tables rot. This one was written against the versions current as of
2026-07; verify against the upstream crates before making a storage decision._

_Reframed for the cloud-sync producer-side buffer target. Comparison tables rot; verify upstream before deciding._

| Feature         | segment-buffer                              | yaque                     | disk_backed_queue      |
| --------------- | ------------------------------------------- | ------------------------- | ---------------------- |
| Target shape    | local spool for cloud sync                  | general queue             | general queue          |
| Process model   | single-process (locked at open)             | SPSC                      | multi-process          |
| Segment files   | zstd+CBOR                                   | raw bytes                 | SQLite                 |
| Ack/delete      | `delete_acked()` (at-least-once)            | `RecvGuard` commit/revert | partial                |
| Crash recovery  | filename-based                              | replay or loss            | SQLite WAL             |
| Compression     | zstd                                        | none                      | none                   |
| Durability knob | 3 policies (Maximal/Segment/Throughput)     | write-through             | SQLite full/normal/off |
| Encryption      | optional (AES-GCM today, XChaCha20 planned) | no                        | no                     |

## Status

**v0.4.2** — the "process debt + semver-leak closure" release. Gates
`fuzz_hooks` behind a `#[cfg]` feature (closes the v0.4.1 semver leak), adds
a CI `loom` job (prevents the v0.4.0-v0.4.1 silent rot of the loom test),
adds 1 new fuzz target (`fuzz_append_all`) and 2 new property tests, and
ships `docs/DOMAIN_LANGUAGE.md` + `docs/CIPHERS.md`. Non-breaking; drop-in
upgrade from v0.4.1.
See [CHANGELOG.md](CHANGELOG.md) for details.
See [FEATURES.md](FEATURES.md), [ROADMAP.md](ROADMAP.md).

**v0.4.1** — the "safety + trust depth" release. Adds `for_each_from` re-entrancy
guard (panics instead of silently deadlocking), `append_all` batch primitive,
`path()`/`config()`/`sync_disk_bytes()` accessors, 2 new fuzz targets, 4 new
property tests, nightly fuzz CI, supply-chain checks (cargo-audit + cargo-deny),
and docs (`docs/PERFORMANCE.md`, `docs/RELEASE.md`, `docs/MSRV.md`).
Non-breaking; drop-in upgrade from v0.4.0.
See [CHANGELOG.md](CHANGELOG.md) for details.
See [FEATURES.md](FEATURES.md), [ROADMAP.md](ROADMAP.md).

**Performance:** the 2026-07-20 PGO session
([docs/perf/2026-07-20_hot-path-flamegraph.md](docs/perf/2026-07-20_hot-path-flamegraph.md))
found that ~66% of `flush` CPU was in zstd re-initialising its ~200 KB `CCtx`
on every `encode_all` call. Pooling a `zstd::bulk::Compressor` on `SegmentBuffer`
made `append/batch_1` **2.07× faster** (15.09 µs → 7.75 µs), with smaller wins
at larger batches. The crate is now substantially faster than v0.1.0 on small
batches; the previous "30–65% regression" framing is obsolete. A `Throughput`
durability policy (above) would remove the per-flush fsync and open a further
large gain on cloud-sync workloads.
_Methodology caveat: single-run, single-machine criterion medians without
statistical noise bars — indicative of direction, not publication-grade. See
[docs/PERFORMANCE.md](docs/PERFORMANCE.md) for methodology and reproduction._

## License

Licensed under the [Apache License, Version 2.0](https://github.com/LarsArtmann/segment-buffer/blob/master/LICENSE).
