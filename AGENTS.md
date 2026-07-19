# AGENTS.md

Concise, enduring context for working in `segment-buffer`. Read once, internalize.

## What this is

A single-crate **Rust library** (not a binary): `SegmentBuffer<T>` — a durable, bounded, MPMC queue that spills in-memory batches to zstd-compressed CBOR segment files with ack-based deletion and filename-based crash recovery. Generic over `T: Serialize + DeserializeOwned + Clone + Send + 'static`. Extracted from monitor365.

## Commands

The `encryption` feature is **off by default**. Most verification commands must be run **twice** — once without features and once with `--features encryption` — because CI does exactly this (see `.github/workflows/ci.yml`).

```bash
# Tests (CONTRIBUTING.md canonical command — runs both default + encryption tests)
cargo test --no-fail-fast --features encryption

# Lint (warnings are hard errors, both in CONTRIBUTING and CI via RUSTFLAGS=-D warnings)
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features encryption -- -D warnings
cargo fmt --all -- --check

# Examples (each is a separate binary)
cargo run --example basic_usage
cargo run --example backpressure
cargo run --example encrypted --features encryption   # REQUIRES the feature flag

# Benchmarks (criterion, 4 separate targets declared in Cargo.toml)
cargo bench --bench bench_append
cargo bench --bench bench_read_from
cargo bench --bench bench_delete_acked
cargo bench --bench bench_recover

# Docs (CI builds with the feature so AesGcmCipher is visible)
cargo doc --no-deps --features encryption

# Property tests (run as part of cargo test, but can be increased)
cargo test --no-fail-fast --features encryption -- property

# Fuzz (requires nightly; see fuzz/README.md)
cargo +nightly fuzz run fuzz_corrupted_read -- -max_total_time=60
cargo +nightly fuzz run fuzz_recovery       -- -max_total_time=60
```

### Nix (reproducible)

```bash
nix develop                 # devShell: rustc/cargo/clippy/rustfmt/rust-analyzer + zstd + pkg-config
nix fmt                     # treefmt: nixfmt + rustfmt (edition 2021, agrees with `cargo fmt`)
nix flake check             # build, test, clippy, fmt, doc — all under the sandbox
nix build .#checks.x86_64-linux.test   # run just the test check
```

## Feature flags

- `default = []`
- `encryption` — pulls in `aes-gcm` + `rand`, exposes `AesGcmCipher`. The `SegmentCipher` **trait** is always available; only the AES-256-GCM impl is gated.

**Known false-positive**: `rust-analyzer` will report `unresolved import segment_buffer::AesGcmCipher` in `examples/encrypted.rs` and a "configured out" hint at `src/lib.rs:32`. This is **not a bug** — rust-analyzer doesn't enable the feature. Real builds with `--features encryption` compile cleanly.

## Architecture & data flow

```
append(item) ─► unflushed: Vec<T>  (in-memory, inside Mutex)
                   │
                   ▼  (batch full OR flush_interval elapsed OR explicit flush())
            take() the batch, compute start_seq/end_seq INSIDE the lock
                   │
                   ▼  (lock released)
            CBOR-serialize  ─►  zstd compress  ─►  [optional cipher.encrypt]   (src/segment.rs)
                   │
                   ▼
            write to seg_*.zst.tmp  ─►  sync_all  ─►  rename to seg_*.zst
                   │
                   ▼  (lock re-acquired)
            approx_disk_bytes += len
```

`read_from(start, limit)` scans on-disk segments first (sorted by `start`), then drains the in-memory pending tail. `delete_acked(seq)` removes every segment whose `end <= seq` and advances `head_seq`.

### Crash recovery (the defining design choice)

**There is no WAL and no metadata database.** State is fully encoded in filenames:

- Pattern: `seg_{start:012}_{end:012}.zst` (12-digit zero-padded, inclusive range)
- On `open()`: `.tmp` files are deleted (incomplete crash writes), then filenames are parsed to rebuild `head_seq` (min start) and `next_seq` (max end + 1).
- Atomic durability comes from the tmp → `sync_all` → `rename` sequence in `write_segment`.

If you change the filename format, `segment::filename` and `segment::parse_filename` (in `src/segment.rs`) are the two sides of the contract — both must stay in sync, and existing on-disk files from monitor365 must still parse.

## Critical concurrency invariant

**`start_seq`, `end_seq`, and the sequence number returned by `append()` must all be computed inside the same mutex lock that takes ownership of `unflushed` / pushes the event.** Do not refactor these into separate lock acquisitions.

This was a real race fixed post-extraction (see CHANGELOG `[0.1.0]` → "Fixed"). The previous code re-read `next_seq` in a second lock and produced corrupted segment filenames under concurrent `append()`.

The `parking_lot::Mutex` is **never held across file I/O** — `flush()` drops it before `write_segment` and re-acquires it only to bump `approx_disk_bytes`. There are no await points; all I/O is synchronous.

## Backpressure / overload policy

The crate **ships no admission policy**. `store_pressure()` returns `approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is just `> 0.9`. Callers define their own priority thresholds — see `examples/backpressure.rs` for the canonical pattern.

## Encryption on-disk format

`AesGcmCipher` writes `[12-byte random nonce][ciphertext + 16-byte GCM tag]` as the segment **payload**. This payload is **byte-compatible with monitor365's `EncryptionKey` segment format** — do not change it without a migration story. `segment::read` rejects encrypted payloads shorter than `NONCE_LEN` (12) as `SegmentError::Integrity` with the offending path.

## Segment file envelope (format evolution)

Every segment written by this crate is wrapped in an 8-byte envelope:

```
offset  bytes  meaning
  0..4    4    magic: ASCII "SBF1"
   4      1    envelope version (currently 1)
  5..8    3    reserved (zero; future: checksum type, compression algo)
  8..          payload (zstd(CBOR), optionally encrypted — the v1 layout)
```

On read, the envelope is **auto-detected**: files starting with `SBF1` are unwrapped; files without the magic are treated as legacy v1 (the original monitor365 format). This makes the envelope strictly additive — no migration needed. The cipher always sees the payload (post-envelope-strip), so cipher byte-compatibility is preserved.

If you need to evolve the format (new checksum, new compression, metadata block), bump `ENVELOPE_VERSION` in `src/segment.rs` and branch on the version in `unwrap_envelope`. Do not change the magic.

## Project layout

```
src/
  lib.rs           SegmentBuffer, SegmentConfig, BufferInner; orchestrates lock + flush policy
  segment.rs       On-disk format: envelope, SegmentRange, filename/parse, encode/decode pipeline, scan, clean_tmp
  cipher.rs        SegmentCipher trait, CipherError, AesGcmCipher (feature-gated impl in `mod private`)
  error.rs         SegmentError (typed: path + phase + reason), Result alias
  tests.rs         `mod tests` — 29 unit tests
  property_tests.rs proptest: filename/payload/envelope bijections, encrypted roundtrip (6 properties)
examples/          basic_usage, backpressure, encrypted (feature-gated)
benches/           4 criterion targets + shared support.rs
fuzz/              cargo-fuzz scaffold (fuzz_corrupted_read, fuzz_recovery); requires nightly
FEATURES.md        Honest capability inventory by status
ROADMAP.md         Long-term direction and explicit non-goals
flake.nix          Reproducible devShell (zstd, pkg-config, Rust toolchain)
```

The split between `lib.rs` (in-memory orchestration + locking) and `segment.rs` (byte-level disk format) is deliberate: the buffer doesn't know how segments are encoded, and the segment module doesn't know about the mutex. `SegmentBuffer`'s private `write_segment`/`read_segment`/`scan_segments`/`segment_path` methods are thin instance-bound wrappers over the stateless `segment::` free functions.

## Code conventions

- `#![warn(missing_docs)]` is on — every public item needs a doc comment.
- Doc comment style uses `# Errors` and `# Example` sections (see `SegmentBuffer::open`).
- Tests use `tempfile::TempDir` and a `test_config(max_size_bytes)` helper with small `max_batch_events: 4` and `flush_interval_secs: 3600` (effectively disables auto-flush).
- The private in-memory field is `unflushed: Vec<T>` (items not yet written to a segment) — distinct from the public `pending_count()` backlog metric. Do not confuse the two.
- The top-level doc example is `#![no_run]`-gated.
- Lint posture is strict: `RUSTFLAGS=-D warnings` plus clippy `-D warnings`.

## CI / MSRV

- Matrix: `ubuntu-latest` + `macos-latest` × `stable` + `1.85`.
- **MSRV is 1.85** (also the `rust-version` in `Cargo.toml`). There is a dedicated `msrv` job that runs `cargo check --all-targets --features encryption` on 1.85.0.
- macOS needs `brew install zstd` (CI does this automatically). Under the Nix devShell (`nix develop`), zstd is provided hermetically so no manual install is needed.
- **`Cargo.lock` is committed** (not gitignored) so Nix flake builds are reproducible. This intentionally overrides the global gitignore; use `git add -f Cargo.lock` if it gets dropped.
