# Roadmap

Long-term direction and raw ideas, not yet refined into actionable work.
Near-term, bounded tasks live in commit history and CHANGELOG; this file holds
the bigger picture.

The v0.1.0 design priorities were **correctness** (the flush concurrency race),
**durability** (filename-based recovery, atomic writes), and **minimal surface**
(no WAL, no async runtime). Future work should preserve those properties.

## Direction

### 1. Async I/O (optional)

All file I/O is synchronous today; the mutex is never held across await points.
An optional async API (`tokio` / `async-std` feature) would let callers integrate
`SegmentBuffer` into async pipelines without offloading every call to
`spawn_blocking`. The hard part is preserving the "mutex never held across I/O"
invariant under cancellation.

### 2. More ciphers

`SegmentCipher` is a trait, so additional AEADs are additive and cheap:

- **ChaCha20-Poly1305** (`chacha20poly1305` crate) under a feature flag.
- **XChaCha20-Poly1305** for extended nonces (no 2^32 message limit per key).

Both must stay self-describing (nonce in-band) to honor the trait contract.

### 3. Pluggable segment store

`SegmentBuffer` is currently bound to the local filesystem. A `SegmentStore`
trait abstracting `write` / `read` / `scan` / `clean_tmp` (currently the private
`segment` module) would enable S3-backed, in-memory, or encrypted-block-device
stores. The filename contract would remain the recovery source of truth.

### 4. Fuzzing & integrity

- A `cargo fuzz` harness targeting `parse_filename`, `encode`/`decode` roundtrips, and recovery from arbitrary on-disk garbage.
- Optional checksum (e.g. Blake3) per segment for detecting bit-rot distinct from cipher authentication failures.

### 5. Observability

- Expose segment count and per-segment sizes via a metrics endpoint or a `stats()` accessor.
- Structured recovery summary (segments found, bytes, head/next seq) is already logged; consider a returned `RecoveryReport` for programmatic use.

## Non-goals (by design)

- **No WAL.** The filename IS the WAL. Adding one would double the durability story and the write amplification.
- **No embedded database.** If you need SQLite-grade querying, use SQLite. This crate optimizes for append/read/ack throughput, not ad-hoc queries.
- **No built-in admission policy.** `store_pressure()` is the signal; the policy is yours (see `examples/backpressure.rs`).
