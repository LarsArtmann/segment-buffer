# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Format envelope:** segment files now carry an 8-byte header (`SBF1` magic + 1-byte version + 3-byte reserved), making the on-disk format forward-evolvable without breaking existing readers. Legacy files without the magic are auto-detected and read transparently — existing monitor365 segments keep working with zero migration. The envelope is stripped before decryption, so cipher byte-compatibility is unchanged.
- **`CipherError` type:** the `SegmentCipher` trait now returns a dedicated lightweight error instead of the full `SegmentError`, so cipher implementations don't need to know about segment paths. The I/O layer enriches it with path context when promoting to `SegmentError::Cipher`.
- **Property-based tests** (`proptest`): the filename bijection, payload encode/decode bijection, envelope wrap/unwrap identity, and full encrypted write→read roundtrip are now verified on 256 randomly generated cases per `cargo test` run.
- **Fuzz scaffold** (`cargo +nightly fuzz`): two targets — `fuzz_corrupted_read` (reading bytes-corrupted segments never panics) and `fuzz_recovery` (opening over a directory of arbitrary garbage never panics). See `fuzz/README.md`.
- `FEATURES.md` — honest feature inventory by status.
- `ROADMAP.md` — long-term direction and explicit non-goals.
- `flake.nix` — reproducible devShell with `zstd`, `pkg-config`, and the Rust toolchain (`nix develop`).
- Shared `benches/support.rs` module consolidating the benchmark helpers previously duplicated across all four criterion targets.

### Changed

- **Typed errors:** `SegmentError::Cbor`, `Cipher`, and `Integrity` variants now carry structured context (`path: PathBuf`, `phase: &'static str` or `reason: &'static str`) instead of opaque `String` payloads. Operators see exactly which file failed and why, without spelunking through logs. Breaking change to the error enum shape (struct variants instead of tuple variants).
- Extracted `src/segment.rs`: the on-disk format (filename contract, envelope, CBOR→zstd→cipher encode/decode pipeline, segment scan, tmp cleanup) now lives in its own module. `SegmentBuffer` focuses purely on in-memory orchestration and locking. No public API change.
- Renamed the private `BufferInner::pending` field to `unflushed` for precision: it holds items not yet written to a segment file, distinct from the public `pending_count()` backlog metric.

### Fixed

- `delete_acked(acked_seq)` no longer under-reports `pending_count()` when called while items are still buffered in memory. `head_seq` is now clamped to the in-memory window so the backlog count stays honest even when the ack cannot remove unflushed items.
- `SegmentBuffer::open` doc corrected: recovery reads filenames only, so it returns `SegmentError::Io` on failure — not `Cbor`/`Integrity` (those surface at `read_from` time).

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

[Unreleased]: https://github.com/LarsArtmann/segment-buffer/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.1.0
