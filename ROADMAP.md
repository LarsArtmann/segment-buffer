# Roadmap

Long-term direction and raw ideas, not yet refined into actionable work.
Near-term, bounded tasks live in [TODO_LIST.md](TODO_LIST.md); this file holds
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

- A `cargo-fuzz` scaffold exists (`fuzz/fuzz_corrupted_read`, `fuzz/fuzz_recovery`) and was verified locally on 2026-07-19 via the Nix `devShells.fuzz` (nightly + `libfuzzer-sys`): `fuzz_corrupted_read` ran 187,811 cases / 60s (392 coverage blocks, zero crashes), `fuzz_recovery` ran 942,719 cases / 60s (zero crashes). **Nightly CI integration landed in v0.4.1** (`.github/workflows/fuzz.yml`); CI proptest analogues run on every `cargo test`. Deeper fuzz targets (envelope edge cases, `parse_filename` over arbitrary UTF-8) are still outstanding.
- Optional checksum (e.g. Blake3) per segment for detecting bit-rot distinct from cipher authentication failures.

### 5. Observability

- The `stats()` accessor shipped in v0.2.0 as a single-lock `BufferStats` snapshot (pending/latest/head/next seq + disk bytes + pressure). v0.3.0 added the `benches/bench_stats.rs` micro-bench proving `stats()` (~12 ns) beats three individual accessors (~31 ns). Richer per-segment metrics (segment count, per-segment size histogram) are still future work.
- Structured recovery summary (`RecoveryReport`) shipped in v0.4.0 via `open_with_report()`. Returns segment_count, head_seq, next_seq, disk_bytes, removed_tmp_files.

### 6. v0.5.0 candidates (next breaking batch)

Deferred breaking changes, batched so users upgrade once. See [TODO_LIST.md](TODO_LIST.md) for the full list. Highlights:

- **`Arc<dyn SegmentCipher>` instead of `Box`** — makes `SegmentConfig` `Clone`.
- **`SegmentIter<'_, T>` lending iterator type** — replaces `for_each_from`'s closure with a true GAT-based iterator for `for (seq, item) in buf.iter_from(0)?` ergonomics.
- **`IoSite` enum for `SegmentError::Io`** — `Dir | Segment(PathBuf) | Unknown` instead of `Option<PathBuf>`.
- **mtime probe for the scan cache** — cheap `stat` to validate against external directory manipulation.

## Non-goals (by design)

- **No WAL.** The filename IS the WAL. Adding one would double the durability story and the write amplification.
- **No embedded database.** If you need SQLite-grade querying, use SQLite. This crate optimizes for append/read/ack throughput, not ad-hoc queries.
- **No built-in admission policy.** `store_pressure()` is the signal; the policy is yours (see `examples/backpressure.rs`).
