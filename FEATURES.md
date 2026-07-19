# Features

Honest inventory of what `segment-buffer` does, by status. Code is the source of
truth; this file tracks reality, not aspirations.

| Status               | Meaning                                                      |
| -------------------- | ------------------------------------------------------------ |
| FULLY_FUNCTIONAL     | Code present and working (tests pass, or exercised in prod). |
| PARTIALLY_FUNCTIONAL | Ships but has documented gaps or edge-case limitations.      |
| PLANNED              | Designed or discussed; no code yet.                          |
| WORTH_CONSIDERING    | Raw idea, not yet designed.                                  |

## Core queue

| Capability                                               | Status               | Notes                                                                                                                                   |
| -------------------------------------------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Durable bounded queue (`SegmentBuffer<T>`)               | FULLY_FUNCTIONAL     | Generic over `T: Serialize + DeserializeOwned + Clone + Send + 'static`. Proven on 597M+ events in monitor365.                          |
| Append with sequence-number assignment (`append`)        | FULLY_FUNCTIONAL     | Sequence numbers computed atomically inside the mutex (concurrency bug fixed; see CHANGELOG).                                           |
| Batch + interval auto-flush (`flush`)                    | FULLY_FUNCTIONAL     | Configurable via `max_batch_events` and `flush_interval_secs`.                                                                          |
| Range read across disk + memory (`read_from`)            | FULLY_FUNCTIONAL     | Merges on-disk segments with the in-memory tail, in ascending sequence order.                                                           |
| Ack-based segment deletion (`delete_acked`)              | PARTIALLY_FUNCTIONAL | Removes flushed segment files only; unflushed in-memory items remain until flushed (documented, count stays honest via head_seq clamp). |
| Backlog size (`pending_count`)                           | FULLY_FUNCTIONAL     | `next_seq - head_seq`; honest even when acks race unflushed items.                                                                      |
| Backpressure metrics (`store_pressure`, `is_overloaded`) | FULLY_FUNCTIONAL     | Ratio of `approx_disk_bytes` to `max_size_bytes`. Admission policy is caller-defined.                                                   |

## Storage format

| Capability                                   | Status               | Notes                                                                                |
| -------------------------------------------- | -------------------- | ------------------------------------------------------------------------------------ |
| zstd + CBOR segment files                    | FULLY_FUNCTIONAL     | `seg_{start:012}_{end:012}.zst`, configurable `compression_level` (1-22).            |
| **Format envelope (`SBF1` magic + version)** | FULLY_FUNCTIONAL     | 8-byte header; forward-evolvable without breaking legacy readers.                    |
| **Legacy file compatibility**                | FULLY_FUNCTIONAL     | Pre-envelope (monitor365) files auto-detected; zero migration.                       |
| Filename-based crash recovery                | FULLY_FUNCTIONAL     | No WAL, no metadata DB. `open()` scans filenames to rebuild `head_seq`/`next_seq`.   |
| Atomic write (tmp → fsync → rename)          | FULLY_FUNCTIONAL     | A crash never leaves a partial segment; `.tmp` debris is cleaned on `open()`.        |
| Crash-recovery limitation                    | PARTIALLY_FUNCTIONAL | Unflushed in-memory items are lost on crash (by design — durability requires flush). |

## Encryption

| Capability                                         | Status           | Notes                                                                              |
| -------------------------------------------------- | ---------------- | ---------------------------------------------------------------------------------- |
| `SegmentCipher` trait (pluggable AEAD)             | FULLY_FUNCTIONAL | Always available; bring any `Send + Sync` encrypt/decrypt impl.                    |
| `AesGcmCipher` (AES-256-GCM, random 12-byte nonce) | FULLY_FUNCTIONAL | Behind the `encryption` feature. Byte-compatible with monitor365's segment format. |

## Concurrency & operations

| Capability                                                      | Status           | Notes                                                                 |
| --------------------------------------------------------------- | ---------------- | --------------------------------------------------------------------- |
| MPMC via `parking_lot::Mutex`                                   | FULLY_FUNCTIONAL | Multiple writers and readers; mutex never held across file I/O.       |
| `tracing` instrumentation (`debug` / `info`)                    | FULLY_FUNCTIONAL | Flush, delete, and recovery events are logged.                        |
| Typed errors with segment-path context                          | FULLY_FUNCTIONAL | `Cbor`/`Cipher`/`Integrity` carry the offending file path.            |
| Criterion benchmarks (append, read_from, delete_acked, recover) | FULLY_FUNCTIONAL | `cargo bench --bench <name>`; shared helpers in `benches/support.rs`. |
| CI matrix (ubuntu/macos × stable/1.85, `-D warnings`)           | FULLY_FUNCTIONAL | Dedicated MSRV (1.85) verification job.                               |

## Testing & trust

| Capability                                | Status               | Notes                                                                          |
| ----------------------------------------- | -------------------- | ------------------------------------------------------------------------------ |
| Unit tests (29) + doc tests (2)           | FULLY_FUNCTIONAL     | CRUD, partial reads, limits, crash recovery, concurrency, error paths.         |
| Property tests (`proptest`, 6 properties) | FULLY_FUNCTIONAL     | Filename bijection, payload bijection, envelope identity, encrypted roundtrip. |
| Fuzz targets (`cargo-fuzz`)               | PARTIALLY_FUNCTIONAL | Scaffold in `fuzz/`; requires nightly; not yet integrated into CI.             |
| Loom concurrency verification             | PLANNED              | Exhaustive schedule check of `append`/`flush`/`delete_acked` not yet in place. |

## Planned / worth considering

See [ROADMAP.md](ROADMAP.md) for long-term direction (async I/O, ChaCha20-Poly1305, pluggable segment store, fuzzing harness).
