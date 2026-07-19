# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_No changes yet — see [0.2.0] below for the most recent release._
Next work is tracked in [TODO_LIST.md](TODO_LIST.md) (v0.2.0 follow-ups first,
then the v0.3.0 breaking batch).

## [0.2.0] - 2026-07-19

This release hardens the format envelope's legacy-detection contract (the
headline correctness fix), makes `CipherError` a real error type with source
chaining, adds several API ergonomics (`len`/`is_empty`/`stats`/`BufferStats`),
and refactors `recover()` so the mutex is no longer held across filesystem
metadata calls. **It is a breaking release** because the `SegmentError` variant
shape and `CipherError` field visibility changed; bump your dependency with
`cargo update -p segment-buffer`.

### Added

- **Format envelope:** segment files now carry an 8-byte header (`SBF1` magic + 1-byte version + 3-byte reserved), making the on-disk format forward-evolvable without breaking existing readers. Legacy files are auto-detected; existing monitor365 segments keep working with zero migration. The envelope is stripped before decryption, so cipher byte-compatibility is unchanged.
- **Envelope hardening (correctness):** legacy detection now requires the `SBF1` magic **and** the 3 reserved bytes to all be zero. This drops the false-positive rate on legacy _encrypted_ files from 2⁻³² per file (~1 silent mis-detection per 7 full monitor365 deployments) to 2⁻⁵⁶ (negligible across the entire 597M-segment corpus). Existing on-disk files written by this crate still parse (they always wrote zeros for the reserved bytes).
- **`CipherError` is now opaque** with private fields and two constructors — `CipherError::msg` (no cause) and `CipherError::with_source` (preserves the underlying AEAD error for `std::error::Error::source()` chaining). The previous `pub String` field is private; cipher implementations no longer need to know about segment paths (the I/O layer still attaches them when promoting to `SegmentError::Cipher`). `AesGcmCipher` now routes its underlying AEAD failure through `source()` instead of flattening it into a `format!`.
- **`len()` and `is_empty()`** standard collection methods on `SegmentBuffer` (semantic aliases of `pending_count() == 0` for idiomatic call sites).
- **`BufferStats` struct + `SegmentBuffer::stats()`** — point-in-time snapshot of `pending_count`, `latest_sequence`, `head_sequence`, `next_sequence`, `approx_disk_bytes`, `max_size_bytes`, and `store_pressure` captured under a single mutex acquisition (no torn reads between calls).
- **`SegmentRange::new(start, end)`** constructor that `debug_assert`s the `start <= end` invariant at construction. Parse-time validation stays loose so legacy files in the wild are surfaced, not dropped.
- **Static `Send + Sync` assertion** on `SegmentBuffer<T>` — turns the documented MPMC thread-safety guarantee into a compile-time contract.
- **`#[must_use]`** on `latest_sequence`, `pending_count`, `len`, `is_empty`, `store_pressure`, `is_overloaded`, and `stats` so accidental discards surface as warnings.
- **Property-based tests** (`proptest`): filename bijection, payload bijection, envelope identity, encrypted roundtrip with a varied key (256 cases per `cargo test`), plus CI-runnable analogues of both `cargo-fuzz` targets (corrupted-segment read never panics; recovery over arbitrary directory contents never panics).
- **Encrypted-legacy read coverage:** the headline monitor365 byte-compatibility guarantee (a `[nonce][ciphertext]` segment file written without the `SBF1` envelope) is now covered by a regression test; previously the entire encrypted-legacy read path had zero coverage.
- **Error-matching doc-test:** `error.rs` now shows how to match on `SegmentError::Cbor { path, phase, .. }` to recover the offending file path and quarantine it.
- **Fuzz scaffold** (`cargo +nightly fuzz`): two targets — `fuzz_corrupted_read` (reading bytes-corrupted segments never panics) and `fuzz_recovery` (opening over a directory of arbitrary garbage never panics). See `fuzz/README.md`.
- `FEATURES.md` — honest feature inventory by status.
- `ROADMAP.md` — long-term direction and explicit non-goals.
- `flake.nix` — reproducible devShell with `zstd`, `pkg-config`, and the Rust toolchain (`nix develop`).
- Shared `benches/support.rs` module consolidating the benchmark helpers previously duplicated across all four criterion targets.

### Changed

- **Typed errors (breaking):** `SegmentError::Cbor`, `Cipher`, and `Integrity` variants now carry structured context (`path: PathBuf`, `phase: &'static str` or `reason: &'static str`) instead of opaque `String` payloads. Operators see exactly which file failed and why, without spelunking through logs.
- **`CipherError` field visibility (breaking):** the previous `pub String` field is now private. Use `Display` or the new constructors.
- Extracted `src/segment.rs`: the on-disk format (filename contract, envelope, CBOR→zstd→cipher encode/decode pipeline, segment scan, tmp cleanup) now lives in its own module. `SegmentBuffer` focuses purely on in-memory orchestration and locking.
- Renamed the private `BufferInner::pending` field to `unflushed` for precision: it holds items not yet written to a segment file, distinct from the public `pending_count()` backlog metric.
- **`recover()` no longer holds the mutex across filesystem metadata calls.** All `fs::metadata` I/O now happens before the lock is taken; the lock is held only long enough to publish the rebuilt `head_seq`/`next_seq`/`approx_disk_bytes`. Restores the invariant that the mutex is never held across file I/O.

### Fixed

- **Envelope false-positive on legacy encrypted files.** As shipped in 0.1.0 unreleased, the envelope magic-only check (2⁻³² false-positive rate) would silently mis-detect roughly 1 in 7 monitor365 deployments' encrypted segments as enveloped, producing spurious `SegmentError::Cipher` errors. Requiring the 3 reserved bytes to also be zero drops the rate to 2⁻⁵⁶.
- `delete_acked(acked_seq)` no longer under-reports `pending_count()` when called while items are still buffered in memory. `head_seq` is now clamped to the in-memory window so the backlog count stays honest even when the ack cannot remove unflushed items.
- `SegmentBuffer::open` doc corrected: recovery reads filenames only, so it returns `SegmentError::Io` on failure — not `Cbor`/`Integrity` (those surface at `read_from` time).
- `fuzz/fuzz_targets/fuzz_recovery.rs` parser had a dead-code `let _ = rest;` and convoluted `split`/`peek` logic; rewrote cleanly.

### Security

- Continuing the 0.1.0 baseline: extracted from monitor365 and proven on 597M+ events in production.

## [0.1.0] - 2026-07-19

### Added

- `SegmentBuffer<T>` — durable bounded queue backed by zstd-compressed CBOR segment files.
  Generic over any `T: Serialize + DeserializeOwned + Clone + Send + 'static`.
- `SegmentConfig` with tunable batch size, flush interval, max disk usage, and compression level.
- `SegmentBuffer::open(dir, config)` constructor with filename-based crash recovery.
- `append()`, `flush()`, `read_from()`, `delete_acked()`, `latest_sequence()`,
  `pending_count()`, `store_pressure()`, `is_overloaded()` public API.
- `SegmentCipher` trait for pluggable at-rest encryption.
- `AesGcmCipher` (AES-256-GCM with random 12-byte nonce prefix) behind the `encryption` feature.
- `SegmentError` (thiserror-based) with `Io`, `Cbor`, `Cipher`, `Integrity` variants.
- 26 unit tests + 2 doc tests covering: basic CRUD, partial reads, limits, crash recovery,
  concurrent writers/readers (10K events, 4 writers + 1 reader), time-based auto-flush,
  error paths (corrupted zstd, truncated encrypted, wrong key, no key), encryption roundtrips,
  and pressure/overload boundary conditions.

### Fixed

- Concurrency bug in `flush()`: sequence numbers (`start_seq`, `end_seq`) are now computed
  atomically inside the mutex lock alongside taking the pending events. Previously, a race
  between concurrent `append()` calls could corrupt segment filenames by computing the
  sequence range from a stale `next_seq` read in a second lock acquisition.
- Same race in `append()`: the returned sequence number is now captured under the same lock
  as the push, not re-read after releasing the lock.

### Security

- Extracted from monitor365 and proven on 597M+ events in production.

[Unreleased]: https://github.com/LarsArtmann/segment-buffer/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.2.0
[0.1.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.1.0
