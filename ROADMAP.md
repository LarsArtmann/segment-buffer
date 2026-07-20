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

`SegmentCipher` is a trait, so additional AEADs are additive and cheap. As of
the v0.5.0 batch, two ciphers ship under the `encryption` feature:

- **`AesGcmCipher`** (AES-256-GCM, 12-byte random nonce) — byte-compatible
  with the original monitor365 segment format.
- **`XChaCha20Poly1305Cipher`** (24-byte extended nonce, no 2³²-message-per-key
  limit, constant-time in software) — installed for new buffers by
  `SegmentConfigBuilder::recommended_cipher(key)`.

Future cipher work is now streaming/incremental encryption (a chunked AEAD
format that bounds memory on large segments and enables early-stop-at-`limit`
reads). This is a format change and is tracked under envelope v2 — see
`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`.

All cipher impls must stay self-describing (nonce in-band) to honor the trait
contract.

### 3. Second `SegmentStore` impl

The `SegmentStore` trait abstraction (local FS, S3, in-memory) **shipped in
the v0.5.0 batch** (`src/store.rs`). Production code constructs a `RealStore`
internally via `open()` / `open_with_report()`; the trait is reachable
externally only under the `loom` Cargo feature (via `open_with_store`), and is
documented as not-stable-semver-surface. A second production impl (S3-backed,
encrypted-block-device, etc.) is **deferred until a concrete consumer exists**
— adding one without a real consumer would be speculative. The filename
contract remains the recovery source of truth regardless of which impl backs
the buffer.

### 4. Fuzzing & integrity

- A `cargo-fuzz` scaffold exists (`fuzz/fuzz_corrupted_read`, `fuzz/fuzz_recovery`, `fuzz/fuzz_parse_filename`, `fuzz/fuzz_envelope`, `fuzz/fuzz_append_all`) and was verified locally on 2026-07-19 via the Nix `devShells.fuzz` (nightly + `libfuzzer-sys`): `fuzz_corrupted_read` ran 187,811 cases / 60s (392 coverage blocks, zero crashes), `fuzz_recovery` ran 942,719 cases / 60s (zero crashes), `fuzz_parse_filename` ran 17M+ cases / 16s (zero crashes), `fuzz_envelope` ran 15M+ cases / 16s (zero crashes), `fuzz_append_all` ran 771k cases / 16s (zero crashes). **Nightly CI integration landed in v0.4.1** (`.github/workflows/fuzz.yml`); CI proptest analogues run on every `cargo test`.
- Optional checksum (e.g. Blake3) per segment for detecting bit-rot distinct from cipher authentication failures.

### 5. Observability

- The `stats()` accessor shipped in v0.2.0 as a single-lock `BufferStats` snapshot (pending/latest/head/next seq + disk bytes + pressure). v0.3.0 added the `benches/bench_stats.rs` micro-bench proving `stats()` (~12 ns) beats three individual accessors (~31 ns). Richer per-segment metrics (segment count, per-segment size histogram) are still future work.
- Structured recovery summary (`RecoveryReport`) shipped in v0.4.0 via `open_with_report()`. Returns segment_count, head_seq, next_seq, disk_bytes, removed_tmp_files.
- v0.4.1 added `path()`, `config()`, `sync_disk_bytes()` accessors and a throughput stress test baseline. The originally reported ~397k events/sec was captured under a mislabeled `Batch(4)` config; the corrected `FlushPolicy::Manual` baseline is ~2.29M events/sec under 8-writer contention (see `docs/perf/2026-07-19_v0.4.1_stress_throughput.md` for the inline correction).

### 6. v0.5.0 batch — SHIPPED in master (pending release tag)

The v0.5.0 "cloud-sync throughput batch" landed in master on 2026-07-20 and
implements the reframing (single-process throughput buffer for cloud sync;
durability-configurable; XChaCha20 recommended cipher; at-least-once
delivery). Release tag is pending explicit approval — see `CHANGELOG.md`
`[Unreleased]` for the full per-item detail and `TODO_LIST.md` for the
per-item status. Highlights:

- **`flock`-based single-process lock** + `SegmentError::Locked`.
- **`DurabilityPolicy` enum** (`Maximal` / `Segment` / `Throughput`).
- **`XChaCha20Poly1305Cipher`** + `recommended_cipher(key)`.
- **`Arc<dyn SegmentCipher + Send + Sync>`** (makes `SegmentConfig` `Clone`).
- **`SegmentIter<'_, T>`** owned-item iterator via `iter_from(start, limit)`.
- **`IoSite` enum** for `SegmentError::Io` (`Dir` / `Segment(PathBuf)` / `Unknown`).
- **mtime probe** for the scan cache (external-manipulation detection).
- **Pooled read-side zstd `Decompressor`** (symmetric to the write-side CCtx pool).

Deferred to v0.6+ / envelope v2 (see
`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`):

- Streaming CBOR deserialise + early-stop at `limit`.
- Per-segment Blake3 checksum.
- Compression-algorithm negotiation, metadata block, streaming cipher, async
  I/O, a second `SegmentStore` impl.

### 7. v0.6+ / envelope v2

Long-term format change. The v2 design ships a 20-byte header (cipher id,
compression id, checksum id, item count, uncompressed size) plus a trailing
Blake3 / CRC32C checksum, and unlocks: streaming deserialise with early-stop
at `limit`, per-segment checksum, and compression-algorithm negotiation. Will
not land until one of those features becomes painful. See
`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`.

## Non-goals (by design)

- **No WAL.** The filename IS the WAL. Adding one would double the durability story and the write amplification.
- **No embedded database.** If you need SQLite-grade querying, use SQLite. This crate optimizes for append/read/ack throughput, not ad-hoc queries.
- **No built-in admission policy.** `store_pressure()` is the signal; the policy is yours (see `examples/backpressure.rs`).
