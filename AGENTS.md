# AGENTS.md

Concise, enduring context for working in `segment-buffer`. Read once, internalize.

## What this is

A single-crate **Rust library** (not a binary): `SegmentBuffer<T>` — a high-throughput **local buffer for cloud sync**. Single-process by design. Spools in-memory batches to zstd-compressed CBOR segment files with ack-based deletion, filename-based crash recovery, configurable durability, and optional encryption. Generic over `T: Serialize + DeserializeOwned + Clone + Send + 'static`. Extracted from monitor365.

**Product positioning (2026-07-20 reframing):** the cloud is the durable layer; this crate is the local throughput buffer in front of it. The README leads with this. The old framing ("durable bounded queue ... crash recovery first") under-sold the actual target use case and over-promised on durability the code doesn't fully deliver (see [Durability model](#durability-model-proposed) below).

## Single-process invariant (design, not yet enforced)

**One owner process per buffer directory.** Multiple threads inside that process are supported (MPMC via `parking_lot::Mutex`). Multiple independent processes opening the same directory is **not supported** — they would race on segment filenames, double-deliver, and corrupt `head_seq`/`next_seq`.

- Today this is a contract, not a mechanism. Nothing prevents a second process from `open()`ing the same directory.
- **Planned for v0.5.0:** `open()` will acquire an exclusive `flock` on a `.segment-buffer.lock` sidecar in the directory, fail fast with a typed error if another process holds it, and release on `Drop` of the `SegmentBuffer`. A subprocess spawned by the owner process that inherits the lock via fd-passing is the user's concern, not the library's.
- If you refactor `open()` or `open_with_report()`, the lock acquisition MUST happen before any filename parsing or state publication.

## At-least-once delivery model

The library provides the substrate for at-least-once delivery; **idempotency lives in the caller's server**.

- `append()` returns the item's stable sequence number (`u64`, monotonic, gap-free across flushes).
- `delete_acked(seq)` is the commit point — it removes every segment whose `end <= seq` and advances `head_seq`.
- Between `read_from(start, ...)` and `delete_acked(start + count - 1)`, a crash leaves the batch on disk. On restart, `read_from(start, ...)` returns it again.
- `read_from` returns `Vec<T>` — items, not `(seq, T)` pairs. The caller tracks `start` and increments it by `batch.len()`. The starting cursor is `buf.stats().head_sequence` after recovery.
- The library does NOT own a cursor file. Cursor persistence is the caller's concern (see [TODO_LIST.md](TODO_LIST.md) — `cursor_file` is a possible future feature; today it is a documented pattern, not an API).
- Only the unflushed in-memory tail is at risk of loss. `flush()` drains it to disk.

## Durability model (proposed)

**Today's behavior** fsyncs the segment file's data but NOT the directory inode after rename (`src/segment.rs` `write()`). This means a host crash within the kernel's dir-inode flush window (~5–30s on ext4/xfs defaults) can leave the renamed file's data on disk but unreachable through the directory. SQLite went through this exact lesson. So today's behavior is **already not fully durable** — the framing isn't "weaken durability for speed," it's "make the tradeoff explicit and configurable."

Proposed `DurabilityPolicy` enum for v0.5.0:

| Policy       | Fsync file | Fsync dir after rename | Worst-case crash loss                            |
| ------------ | ---------- | ---------------------- | ------------------------------------------------ |
| `Maximal`    | yes        | yes                    | last in-flight flush only                        |
| `Segment`    | yes        | no                     | rename window (~5–30s of flushes) — today        |
| `Throughput` | no         | no                     | entire OS dirty window (~30s) — cloud is durable |

- `Throughput` is the correct default for cloud-sync deployments where the cloud endpoint holds the durable copy and the local disk is a throughput buffer.
- `Maximal` is for standalone-queue deployments where this buffer is the last copy.
- Backward compatibility: default stays `Segment` for one release after the enum lands, then flips to `Throughput` with a deprecation note. This is a policy decision pending user input — see open questions in the vision chat (2026-07-20).
- Implementation: thread `policy: DurabilityPolicy` into `segment::write`. Branch there (not a callback). `Copy` enum, no allocation.
- The `Mutex<Compressor>` invariant ("never held across I/O") is preserved: the fsync happens after compression is done and the mutex is released.

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
            prepend 8-byte SBF1 envelope  ─►  write to seg_*.zst.tmp
                   │                          ─►  sync_all  ─►  rename to seg_*.zst
                   ▼  (lock re-acquired)
            approx_disk_bytes += len
```

`read_from(start, limit)` scans on-disk segments first (sorted by `start`), then drains the in-memory pending tail. `delete_acked(seq)` removes every segment whose `end <= seq` and advances `head_seq`. Read path strips the envelope (auto-detecting legacy v1 files) before decryption.

### Crash recovery (the defining design choice)

**There is no WAL and no metadata database.** State is fully encoded in filenames:

- Pattern: `seg_{start:012}_{end:012}.zst` (12-digit zero-padded, inclusive range)
- On `open()`: `.tmp` files are deleted (incomplete crash writes), then filenames are parsed to rebuild `head_seq` (min start) and `next_seq` (max end + 1).
- Atomic durability comes from the tmp → `sync_all` → `rename` sequence in `write_segment`.

If you change the filename format, `segment::filename` and `segment::parse_filename` (in `src/segment.rs`) are the two sides of the contract — both must stay in sync, and existing on-disk files from monitor365 must still parse.

## Critical concurrency invariant

**`start_seq`, `end_seq`, and the sequence number returned by `append()` must all be computed inside the same mutex lock that takes ownership of `unflushed` / pushes the event.** Do not refactor these into separate lock acquisitions.

This was a real race fixed post-extraction (see CHANGELOG `[0.1.0]` → "Fixed"). The previous code re-read `next_seq` in a second lock and produced corrupted segment filenames under concurrent `append()`.

The `parking_lot::Mutex` is **never held across file I/O** — `flush()` drops it before `write_segment` and re-acquires it only to bump `approx_disk_bytes`; `recover()` collects all segment metadata (file sizes via `fs::metadata`, head/next seq from filenames) before taking the lock once to publish the rebuilt state. There are no await points; all I/O is synchronous.

## Backpressure / overload policy

The crate **ships no admission policy**. `store_pressure()` returns `approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is just `> 0.9`. Callers define their own priority thresholds — see `examples/backpressure.rs` for the canonical pattern.

## Encryption on-disk format

`AesGcmCipher` writes `[12-byte random nonce][ciphertext + 16-byte GCM tag]` as the segment **payload**. This payload is **byte-compatible with monitor365's `EncryptionKey` segment format** — do not change it without a migration story. `segment::read` rejects encrypted payloads shorter than `NONCE_LEN` (12) as `SegmentError::Integrity` with the offending path.

**Cipher evolution (2026-07-20 direction):**

- **AES-256-GCM** stays — legacy byte-compat with monitor365 is a hard constraint. Not deprecated.
- **XChaCha20-Poly1305** is the planned default for new buffers. Extended 24-byte nonce eliminates the 2³²-message limit per key that AES-GCM's 12-byte nonce imposes (relevant at high segment counts), and ChaCha20 avoids AES-NI timing side-channels on hosts without hardware acceleration (ARM, older CPUs). The `SegmentCipher` trait is already algorithm-agnostic — adding `XChaChaCipher` is a feature-gated impl, not a trait change.
- **Streaming/incremental cipher** is a long-term direction. Today the whole segment is buffered (CBOR → zstd → encrypt as a blob); a streaming AEAD (e.g. RFC 8450 chunked format) would bound memory on large segments and enable early-stop-at-`limit` reads. Cost: format change. Likely v0.6+.
- A future `SegmentConfigBuilder::recommended_cipher()` helper may pick XChaCha20 when available and AES-GCM as fallback. This is a v0.5.0 decision pending user input.

When adding a cipher: feature-gate it, expose it under `src/cipher.rs` alongside `AesGcmCipher`, add property tests (roundtrip, tamper, short-payload), and document the on-disk format here.

## Segment file envelope (format evolution)

Every segment written by this crate is wrapped in an 8-byte envelope:

```
offset  bytes  meaning
  0..4    4    magic: ASCII "SBF1"
   4      1    envelope version (currently 1)
  5..8    3    reserved (zero; future: checksum type, compression algo)
  8..          payload (zstd(CBOR), optionally encrypted — the v1 layout)
```

On read, the envelope is **auto-detected**: a file is treated as enveloped only when the magic matches **and** the 3 reserved bytes are all zero. Requiring the reserved bytes is what makes the false-positive rate on legacy encrypted files (whose first 7 bytes are random AEAD nonce) **2⁻⁵⁶ per file — negligible even across the full 597M-segment monitor365 corpus**. Files without both conditions are treated as legacy v1 (the original monitor365 format). This makes the envelope strictly additive — no migration needed. The cipher always sees the payload (post-envelope-strip), so cipher byte-compatibility is preserved.

If you need to evolve the format (new checksum, new compression, metadata block), bump `ENVELOPE_VERSION` in `src/segment.rs` and branch on the version in `unwrap_envelope`. The reserved-bytes-zero invariant must keep holding for any new v1-compatible version; repurpose them only when bumping to a version that is allowed to refuse legacy detection.

## Project layout

```
src/
  lib.rs           SegmentBuffer, SegmentConfig, BufferStats, BufferInner; orchestrates lock + flush policy
  segment.rs       On-disk format: envelope, SegmentRange, filename/parse, encode/decode pipeline, scan, clean_tmp
  cipher.rs        SegmentCipher trait, CipherError (opaque: private fields + ErrorExt upcast for MSRV 1.85), AesGcmCipher (feature-gated impl in `mod private`)
  error.rs         SegmentError (typed: path + phase + reason), Result alias
  tests.rs         `mod tests` — 32 unit tests
  property_tests.rs proptest: filename/payload/envelope bijections, encrypted roundtrip, corrupted/recovery fuzz analogues (8 properties)
examples/          basic_usage, backpressure, encrypted (feature-gated)
benches/           4 criterion targets + shared support.rs
fuzz/              cargo-fuzz scaffold (fuzz_corrupted_read, fuzz_recovery); requires nightly
FEATURES.md        Honest capability inventory by status
TODO_LIST.md       Short/mid-term improvement tasks with status
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
- **Local MSRV verification (verified 2026-07-19):** `nix develop .#msrv -c cargo check --all-targets --features encryption` plus `cargo test --no-fail-fast --features encryption` and `cargo clippy --all-targets --features encryption -- -D warnings` were all run on Rust 1.85.0 via the `rust-overlay`-pinned `devShells.msrv` in `flake.nix`. All three returned exit 0 with no warnings. The MSRV claim is now backed by local evidence, not just CI.
- **Pre-1.86 trait-upcasting workaround:** `CipherError::source()` uses a private `ErrorExt` trait in `src/cipher.rs` to upcast `Arc<dyn ErrorExt + Send + Sync>` → `&dyn Error` because Rust 1.85 cannot coerce `dyn ErrorExt` to `dyn Error` directly. Once the MSRV moves to 1.86+, this trait can be deleted and `source()` simplified to `self.source.as_deref()` (tracked in TODO_LIST.md under the v0.3.0 batch).
- macOS needs `brew install zstd` (CI does this automatically). Under the Nix devShell (`nix develop`), zstd is provided hermetically so no manual install is needed.
- **`Cargo.lock` is committed** (not gitignored) so Nix flake builds are reproducible. This intentionally overrides the global gitignore; use `git add -f Cargo.lock` if it gets dropped.
- **Loom concurrency testing** (`tests/loom.rs`): run with `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`. Covers only the in-memory append + `stats()` snapshot path — loom does not model the filesystem, so `flush`/`delete_acked`/`read_from`/`recover` are covered by the stress test `concurrency_4_writers_1_reader_10k_events` in `src/tests.rs` instead. Use `--release` — loom's schedule enumeration is slow in debug.

## Verification discipline (hard rules)

These rules were installed after three consecutive same-day sessions produced
self-reviews that claimed success without running the verification gate,
fabricated working-tree state, and invented baselines. They are non-negotiable
for any future agent (or human) working in this repo.

1. **Never describe working-tree state without a fresh `git status` in the same message.** "8 files staged", "working tree clean", "all committed" — all of these require a literal `git status` invocation in the current response. Re-running `git status` costs 100 ms; the cost of being wrong is a misleading commit message, a broken push, or a false release claim.
2. **Never invent baselines.** Health scores, perf numbers, "was X, now Y", "previously N tests" — if you cannot cite the source of the "previous" value, say "first audit" or "no prior baseline" instead. Numbers without provenance are lies with extra steps.
3. **Line-number citations are banned.** Cite section names, item text, or commit hashes. Line numbers shift the moment any file above the citation is edited; they rot in the same session that wrote them.
4. **Run the verification gate before declaring work done.** `cargo fmt --all -- --check` + `cargo clippy --all-targets --features encryption -- -D warnings` + `cargo test --no-fail-fast --features encryption` + `cargo doc --no-deps --features encryption`. Any claim that "tests pass" or "the build is green" must rest on a literal run of these in the current session, with the exit codes captured.
5. **The supply-chain gate is BOTH `cargo audit` AND `cargo deny check`.** They pull from different advisory sources in edge cases. Running only one is not equivalent to running both. The CI `supply-chain` job runs both; the local pre-commit gate must too.
6. **The loom gate is `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`.** Files gated by `#![cfg(loom)]` are invisible to `cargo test` by default and silently rot without this explicit invocation. The CI `loom` job enforces it.
7. **Concurrency tests must use `FlushPolicy::Manual`.** With `Batch(4)` the stress test creates 20 000 segment files (80 000 items / 4), causing pathological I/O under parallel test execution that hung CI for hours. `Manual` keeps items in-memory so the test stresses mutex contention, not the filesystem.
8. **Doctests that need `--features encryption` must be cfg-gated.** A `rust,no_run` code fence referencing `AesGcmCipher` fails to compile under `cargo test` (default features). Use the hidden `#[cfg(feature = "encryption")] fn main() {}` pattern — see the README encryption example.
9. **Before `git tag` for a release, the most recent CI + Nix runs on the target branch must be green.** Run `gh run list --limit 4` and confirm every run on the branch you are tagging shows `success`. Local-only verification (rule 4) is NOT sufficient: v0.4.1 and v0.4.2 both shipped with a "verification gate" that never checked GitHub Actions, leaving CI broken for 48+ hours while status reports claimed "all green". A release tag on an unverified commit is a lie of omission.

### Session-end checklist

Before writing any closing summary, status report, or "done" claim:

- [ ] `git status` — clean? Or have I explained every modified/untracked file?
- [ ] `git log --format='%h %ci %s' -10` — do the commits match what I think I did?
- [ ] Verification gate run with non-zero exit codes captured (see rule 4)?
- [ ] Every doc claim that says "passes"/"verified"/"green" cites a commit hash or a literal command output in this session?
- [ ] No fabricated numbers — every "was X / now Y" has a citation or has been rewritten to "first audit" / "no baseline"?
- [ ] TODO_LIST updated for anything completed or partially completed this session?
- [ ] **Did I ship a release?** If yes: did the user explicitly approve the release scope? Never ship breaking changes without explicit approval. Never ship two releases in the same day without a soak period.
- [ ] **Before tagging a release: did `gh run list --limit 4` show the latest CI + Nix runs on the target branch as `success`?** (Rule 9.) A local-only green is not a release-ready green.
- [ ] **Did I draft the GitHub release notes BEFORE pushing the tag?** A tag-without-release window (even 2 minutes) breaks link checkers and confuses downstream consumers.

If any of these cannot be checked, the closing summary must say so explicitly. "Working tree clean" without `git status` in the same response is a process failure, not a shorthand.
