# AGENTS.md

Concise, enduring context for working in `segment-buffer`. Read once, internalize.

## What this is

A single-crate **Rust library** (not a binary): `SegmentBuffer<T>` — a high-throughput **local buffer for cloud sync**. Single-process by design. Spools in-memory batches to zstd-compressed CBOR segment files with ack-based deletion, filename-based crash recovery, configurable durability, and optional encryption. Generic over `T: Serialize + DeserializeOwned + Clone + Send` (`'static` is implied by `DeserializeOwned`, not required by the mutex — see TODO_LIST.md "Investigation"). Extracted from monitor365.

**Product positioning (2026-07-20 reframing):** the cloud is the durable layer; this crate is the local throughput buffer in front of it. The README leads with this. The old framing ("durable bounded queue ... crash recovery first") under-sold the actual target use case and over-promised on durability the code doesn't fully deliver (see [Durability model](#durability-model-proposed) below).

## Single-process invariant (enforced since v0.5.0)

**One owner process per buffer directory.** Multiple threads inside that process are supported (MPMC via `parking_lot::Mutex`). Multiple independent processes opening the same directory is **rejected** — they would race on segment filenames, double-deliver, and corrupt `head_seq`/`next_seq`.

- **Mechanism:** `open()` acquires an exclusive `flock` (via `fs4::FileExt::try_lock`) on a `.segment-buffer.lock` sidecar in the directory, fail-fast with [`SegmentError::Locked`] if another process holds it. The lock file handle is held in the `lock_file: Option<std::fs::File>` field on `SegmentBuffer` and released on `Drop` (explicit `unlock()` plus fd close as a belt-and-braces guarantee).
- **Cross-platform:** `fs4` uses `flock(2)` on Linux/macOS, `LockFileEx` on Windows. Pure-Rust via `rustix`, no `libc` dep.
- **Lock failure mode:** `Err(SegmentError::Locked)` — no block, no timeout. Callers retry on their own schedule if they want.
- **Loom tests bypass the lock** via `open_with_store` (loom does not model the filesystem, and a real lock file inside `loom::model` would deadlock). The `open_with_store` constructor passes `lock_file: None`.
- **Subprocess spawned by the owner** that inherits the lock fd via fd-passing is the user's concern, not the library's.
- If you refactor `open()` or `open_with_report()`, the lock acquisition MUST happen after `create_dir_all` and BEFORE any filename parsing or state publication.

## At-least-once delivery model

The library provides the substrate for at-least-once delivery; **idempotency lives in the caller's server**.

- `append()` returns the item's stable sequence number (`u64`, monotonic, gap-free across flushes).
- `delete_acked(seq)` is the commit point — it removes every segment whose `end <= seq` and advances `head_seq`.
- Between `read_from(start, ...)` and `delete_acked(start + count - 1)`, a crash leaves the batch on disk. On restart, `read_from(start, ...)` returns it again.
- `read_from` returns `Vec<T>` — items, not `(seq, T)` pairs. The caller tracks `start` and increments it by `batch.len()`. The starting cursor is `buf.stats().head_sequence` after recovery.
- The library does NOT own a cursor file. Cursor persistence is the caller's concern (see [Layer split vs monitor365](#layer-split-vs-monitor365)).
- Only the unflushed in-memory tail is at risk of loss. `flush()` drains it to disk.

## Layer split vs monitor365

segment-buffer is the **producer-side local buffer**. Everything cloud-facing lives upstream. This split was verified against monitor365's source on 2026-07-20 after the user flagged potential scope creep. Respect it: do not pull upstream concerns into this crate.

| Concern                                            | Owner                 | Location                          |
| -------------------------------------------------- | --------------------- | --------------------------------- |
| Queue: `append`/`flush`/`read_from`/`delete_acked` | **segment-buffer**    | `src/lib.rs`                      |
| Sequence numbers (stable, monotonic, gap-free)     | **segment-buffer**    | `SegmentBuffer::append` return    |
| Segment file format, crash recovery                | **segment-buffer**    | `src/segment.rs`                  |
| `SyncCursor` (newtype around `u64`)                | **monitor365**        | `cloud-client/src/sync_cursor.rs` |
| Cursor persistence (SQLite + WAL)                  | **monitor365**        | `cloud-client/src/sync_state.rs`  |
| Cloud sync orchestration loop                      | **monitor365**        | `cli/src/cloud_sync.rs`           |
| Server-side idempotency (`event_id` dedup)         | **monitor365 server** | (not in client)                   |

**Consequences for this crate:**

- **No cursor file.** The cursor is the consumer's concern. monitor365 stores it in SQLite with its own fsync discipline; pulling cursor persistence into segment-buffer would tangle two durability models, re-introduce the per-ack fsync cost `Throughput` removes, and mis-model the per-device vs per-directory cardinality. See TODO_LIST.md — the cursor-file item is REJECTED with rationale.
- **No backpressure policy.** The crate ships `store_pressure()` / `is_overloaded()` metrics only. The decision to block, sample, drop, or crash on disk-full is the upstream consumer's. segment-buffer just makes the metrics available. See `examples/backpressure.rs` for the canonical pattern.
- **No cloud client.** No HTTP, no retry policy, no auth. The drain loop is the consumer's `read_from → upload → delete_acked` cycle.
- **No server-side dedup.** Idempotency on `(producer_id, seq)` lives in the consumer's server. The library delivers at-least-once; the server makes it effectively-once.

If a proposed feature pulls any of the above into segment-buffer, reject it and document it as upstream's concern.

**Future cloud-sync extraction.** Cloud-sync may one day be extracted from monitor365 into its own crate. That extracted crate would sit _between_ segment-buffer and the cloud — it would consume segment-buffer, not be merged with it. segment-buffer stays the focused producer-side local buffer regardless. Do not use "cloud-sync will eventually be extracted" as a rationale for pulling sync logic, cursors, retry policy, or HTTP into this crate — that is scope creep in either direction.

## Durability model (shipped in v0.5.0)

**Today's default behavior** (`DurabilityPolicy::Segment`, what the crate has always done) fsyncs the segment file's data but NOT the directory inode after rename (`src/segment.rs` `write()` → `src/store.rs` `RealStore::write_atomic`). This means a host crash within the kernel's dir-inode flush window (~5–30s on ext4/xfs defaults) can leave the renamed file's data on disk but unreachable through the directory. SQLite went through this exact lesson. So today's behavior is **already not fully durable** — the framing isn't "weaken durability for speed," it's "make the tradeoff explicit and configurable."

The `DurabilityPolicy` enum shipped in v0.5.0:

| Policy       | Fsync file | Fsync dir after rename | Worst-case crash loss                            |
| ------------ | ---------- | ---------------------- | ------------------------------------------------ |
| `Maximal`    | yes        | yes                    | last in-flight flush only                        |
| `Segment`    | yes        | no                     | rename window (~5–30s of flushes) — pre-v0.5.0   |
| `Throughput` | no         | no                     | entire OS dirty window (~30s) — cloud is durable |

- `Throughput` is the correct default for cloud-sync deployments where the cloud endpoint holds the durable copy and the local disk is a throughput buffer.
- `Maximal` is for standalone-queue deployments where this buffer is the last copy.
- Backward compatibility: default stays `Segment` for one release after the enum lands, then flips to `Throughput` with a deprecation note.
- Implementation: `policy: DurabilityPolicy` is threaded through `SegmentStore::write_atomic`; the trait signature now takes it as a third parameter. `RealStore::write_atomic` branches on it. The loom `MockStore` accepts it for signature compatibility and ignores it (loom does not model fsync). The policy is a `Copy` enum, no allocation.
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

# Supply-chain publisher provenance (INFORMATIONAL — not part of the gate).
# `cargo audit` + `cargo deny` flag vulnerabilities and policy violations but
# neither shows WHO can publish the crates in the tree. This lists every
# crates.io account with publish rights over the dependency graph — run it
# when reviewing a Cargo.lock bump to spot unexpected new publishers or
# ownership transfers (the npm-style compromised-maintainer vector). The
# weekly `.github/workflows/supply-chain-report.yml` job runs the same thing.
cargo install cargo-supply-chain --locked
cargo supply-chain publishers
cargo supply-chain publishers --features encryption
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
            segment::encode_segment  ─►  CBOR → zstd → [optional cipher.encrypt] → prepend 8-byte SBF1 envelope   (pure, src/segment.rs)
                   │
                   ▼
            store.write_atomic(range, bytes)  ─►  tmp → sync_all → rename to seg_*.zst   (src/store.rs)
                   │
                   ▼  (lock re-acquired)
            approx_disk_bytes += len
```

`read_from(start, limit)` scans on-disk segments first (sorted by `start`), then drains the in-memory pending tail. `delete_acked(seq)` removes every segment whose `end <= seq` and advances `head_seq`. Read path calls `store.read_bytes` then `segment::decode_segment`, which strips the envelope (auto-detecting legacy v1 files) before decryption.

### Three-layer separation (since 2026-07-20)

The crate has a deliberate three-layer split:

| Layer             | Module           | Knows about                                                                                                                                                                          |
| ----------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Format**        | `src/segment.rs` | Bytes only — envelope, filename, CBOR/zstd/cipher pipeline. Pure functions, zero `std::fs`.                                                                                          |
| **I/O**           | `src/store.rs`   | The `SegmentStore` trait + `RealStore` impl. Owns the on-disk representation: `create_dir_all`, `scan`, `clean_tmp`, `segment_size`, `remove_segment`, `write_atomic`, `read_bytes`. |
| **Orchestration** | `src/lib.rs`     | `SegmentBuffer`: mutex, flush policy, sequence-number invariants. Holds `Arc<dyn SegmentStore + Send + Sync>` and delegates every filesystem call through it.                        |

`open()` / `open_with_report()` construct `RealStore` internally; the trait is reachable externally only under the `loom` Cargo feature (via `SegmentBuffer::open_with_store`). The trait-object approach (~5 ns vtable cost per I/O call, negligible next to zstd+CBOR+file I/O) was chosen over a type parameter `SegmentBuffer<T, S: SegmentStore>` because the latter would force every example, bench, fuzz target, and doc test to spell `<T, RealStore>` (~20 callsites of churn for a testing-only improvement).

### Crash recovery (the defining design choice)

**There is no WAL and no metadata database.** State is fully encoded in filenames:

- Pattern: `seg_{start:012}_{end:012}.zst` (12-digit zero-padded, inclusive range)
- On `open()`: `.tmp` files are deleted (incomplete crash writes), then filenames are parsed to rebuild `head_seq` (min start) and `next_seq` (max end + 1).
- Atomic durability comes from the tmp → `sync_all` → `rename` sequence in `write_segment`.

If you change the filename format, `segment::filename` and `segment::parse_filename` (in `src/segment.rs`) are the two sides of the contract — both must stay in sync, and existing on-disk files from monitor365 must still parse.

## Critical concurrency invariant

**`start_seq`, `end_seq`, and the sequence number returned by `append()` must all be computed inside the same mutex lock that takes ownership of `unflushed` / pushes the event.** Do not refactor these into separate lock acquisitions.

This was a real race fixed post-extraction (see CHANGELOG `[0.1.0]` → "Fixed"). The previous code re-read `next_seq` in a second lock and produced corrupted segment filenames under concurrent `append()`.

The `parking_lot::Mutex` is **never held across file I/O** — `flush()` drops it before `write_segment` and re-acquires it only to bump `approx_disk_bytes`; `recover()` collects all segment metadata (file sizes via `store.segment_size`, head/next seq from filenames) before taking the lock once to publish the rebuilt state. There are no await points; all I/O is synchronous. The `SegmentStore` trait object is invoked outside the mutex exactly as `std::fs` was before the refactor.

### `delete_acked` + `append` interleaving (loom-proven since 2026-07-20)

The clamp at the end of `delete_acked`:

```rust
let pending_start = inner.next_seq.saturating_sub(inner.unflushed.len() as u64);
inner.head_seq = new_head.unwrap_or(inner.next_seq).min(pending_start);
```

is exhaustively proven correct across every schedule of two threads by the loom tests `delete_acked_during_append_never_loses_head`, `delete_acked_past_flush_boundary_with_concurrent_append`, `stats_snapshot_consistent_under_delete_plus_append`, and `delete_acked_idempotent_under_concurrent_append` in `tests/loom.rs`. The proof depends on a `MockStore` (loom-aware in-memory stub) being injected via `open_with_store`; the production `RealStore` shares the same trait, so the proof transfers. The stress test `concurrency_4_writers_1_reader_10k_events` covers the same interleaving _statistically_; loom covers it _exhaustively_.

## Backpressure / overload policy

The crate **ships no admission policy**. `store_pressure()` returns `approx_disk_bytes / max_size_bytes ∈ [0.0, 1.0]`; `is_overloaded()` is just `> 0.9`. Callers define their own priority thresholds — see `examples/backpressure.rs` for the canonical pattern.

## Encryption on-disk format

`AesGcmCipher` writes `[12-byte random nonce][ciphertext + 16-byte GCM tag]` as the segment **payload**. This payload is **byte-compatible with monitor365's `EncryptionKey` segment format** — do not change it without a migration story. `segment::decode_segment` rejects encrypted payloads shorter than `NONCE_LEN` (12) as `SegmentError::Integrity` with the offending path.

`XChaCha20Poly1305Cipher` (shipped v0.5.0, behind the same `encryption` feature) writes `[24-byte random nonce][ciphertext + 16-byte Poly1305 tag]`. The 24-byte nonce eliminates the 2³²-message per-key limit of AES-GCM's 12-byte nonce, and ChaCha20 is constant-time in software (no AES-NI dependency). This is the cipher `SegmentConfigBuilder::recommended_cipher(key)` installs for new buffers; legacy AES-GCM segments still decrypt through `AesGcmCipher`. The two formats are byte-distinguishable only by which cipher the buffer was opened with (no envelope marker for the cipher type today — see the envelope v2 design doc for the migration path).

**Cipher evolution (2026-07-20 direction, partly shipped):**

- **AES-256-GCM** stays — legacy byte-compat with monitor365 is a hard constraint. Not deprecated.
- **XChaCha20-Poly1305** shipped in v0.5.0 as the recommended cipher for new buffers via `recommended_cipher()`. Same `SegmentCipher` trait, feature-gated impl alongside AES-GCM.
- **Streaming/incremental cipher** is a long-term direction. Today the whole segment is buffered (CBOR → zstd → encrypt as a blob); a streaming AEAD (e.g. RFC 8450 chunked format) would bound memory on large segments and enable early-stop-at-`limit` reads. Cost: format change. Likely v0.6+.

When adding a cipher: feature-gate it, expose it under `src/cipher.rs` alongside `AesGcmCipher` and `XChaCha20Poly1305Cipher`, add property tests (roundtrip, tamper, short-payload), and document the on-disk format here.

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
  lib.rs           SegmentBuffer, SegmentConfig, BufferStats, BufferInner; orchestrates lock + flush policy + Arc<dyn SegmentStore>
  segment.rs       On-disk format (PURE, no I/O): envelope, SegmentRange, filename/parse, encode_segment, decode_segment, encode_payload, decode_payload, wrap/unwrap_envelope
  store.rs         SegmentStore trait + RealStore impl: the I/O boundary. create_dir_all / scan / clean_tmp / segment_size / remove_segment / write_atomic / read_bytes
  cipher.rs        SegmentCipher trait, CipherError (opaque: private fields + `Arc<dyn Error + Send + Sync>` source for chaining), AesGcmCipher (feature-gated impl in `mod private`)
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

- Matrix: `ubuntu-latest` + `macos-latest` × `stable` + `1.86`.
- **MSRV is 1.86** (also the `rust-version` in `Cargo.toml`). There is a dedicated `msrv` job that runs `cargo check --all-targets --features encryption` on 1.86.0.
- **Local MSRV verification:** `nix develop .#msrv -c cargo check --all-targets --features encryption`. The `devShells.msrv` in `flake.nix` pins `rust-bin.stable."1.86.0"` via rust-overlay.
- **MSRV consistency guard:** `scripts/check-msrv.sh` asserts that `Cargo.toml rust-version`, `ci.yml` matrix + msrv job, `flake.nix` msrv shell pin, and `docs/MSRV.md` headline all agree. Run by the CI `msrv-consistency` job to prevent drift.
- macOS needs `brew install zstd` (CI does this automatically). Under the Nix devShell (`nix develop`), zstd is provided hermetically so no manual install is needed.
- **`Cargo.lock` is committed** (not gitignored) so Nix flake builds are reproducible. This intentionally overrides the global gitignore; use `git add -f Cargo.lock` if it gets dropped.
- **Loom concurrency testing** (`tests/loom.rs`): run with `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`. Covers only the in-memory append + `stats()` snapshot path — loom does not model the filesystem, so `flush`/`delete_acked`/`read_from`/`recover` are covered by the stress test `concurrency_4_writers_1_reader_10k_events` in `src/tests.rs` instead. Use `--release` — loom's schedule enumeration is slow in debug.

## Releases

**All 8 versions (0.1.0 through 0.5.1) are published on BOTH crates.io and GitHub releases** (synced 2026-07-20). Before that sync, three crates.io versions (0.2.0/0.3.0/0.4.0) and one GitHub release (v0.1.0) were missing — the gap existed because `publish.yml` auto-publishes to crates.io on tag push but does NOT create GitHub releases, and the workflow had not been set up yet when the early tags were pushed.

Two surfaces, two responsibilities:

- **crates.io:** `.github/workflows/publish.yml` publishes automatically on `git push origin v*.*.*` (needs `CARGO_REGISTRY_TOKEN` secret). To backfill a missing version manually, `git worktree add --detach <dir> <tag>` then `cargo publish --features encryption` from that worktree (the tag's `Cargo.toml` version must match the tag). Verify with `cargo publish --dry-run --features encryption` first.
- **GitHub releases:** NOT automated by any workflow. They are created manually. `gh release create` fails on this repo demanding the `workflow` scope (a false-positive scope check); use `gh api --method POST repos/LarsArtmann/segment-buffer/releases -f tag_name=vX.Y.Z -f name=... -f body=...` instead (only `repo` scope needed). Do NOT pass `target_commitish` pointing at a tag name — it 404s; the tag is resolved from `tag_name` alone.

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
6. **The loom gate is `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`.** Files gated by `#![cfg(loom)]` are invisible to `cargo test` by default and silently rot without this explicit invocation. The CI `loom` job enforces it. Coverage (as of 2026-07-20): the in-memory hot path (`append`/`pending_count`/`latest_sequence`/`stats`/`append_all`) AND the `delete_acked` + `append` interleaving (4 tests, exhaustively enumerating every schedule of two threads via the `MockStore` injected through `open_with_store`). `flush`/`recover`/`read_from` still touch byte-level encode/decode that loom has no interest in enumerating; their concurrency contracts stay covered _statistically_ by the stress test `concurrency_4_writers_1_reader_10k_events`.
7. **Concurrency tests must use `FlushPolicy::Manual`.** With `Batch(4)` the stress test creates 20 000 segment files (80 000 items / 4), causing pathological I/O under parallel test execution that hung CI for hours. `Manual` keeps items in-memory so the test stresses mutex contention, not the filesystem.
8. **Doctests that need `--features encryption` must be cfg-gated.** A `rust,no_run` code fence referencing `AesGcmCipher` fails to compile under `cargo test` (default features). Use the hidden `#[cfg(feature = "encryption")] fn main() {}` pattern — see the README encryption example.
9. **Before `git tag` for a release, the most recent CI + Nix runs on the target branch must be green.** Run `gh run list --limit 4` and confirm every run on the branch you are tagging shows `success`. Local-only verification (rule 4) is NOT sufficient: v0.4.1 and v0.4.2 both shipped with a "verification gate" that never checked GitHub Actions, leaving CI broken for 48+ hours while status reports claimed "all green". A release tag on an unverified commit is a lie of omission.
10. **CI-red is a stop-work condition.** If `gh run list --limit 4` shows red on the target branch, the first work item is "turn it green," not "add features on top." Local-only green is never a green claim; check `gh run list` before ANY "done" claim, not just before releases. The investigation sweep of 2026-07-20 documented this exact failure mode: a session claimed "all gates green" while CI was on its 5th consecutive red run due to MSRV drift the session had noticed and dismissed as "out of scope."

### Session-end checklist

Before writing any closing summary, status report, or "done" claim:

- [ ] `git status` — clean? Or have I explained every modified/untracked file?
- [ ] `git log --format='%h %ci %s' -10` — do the commits match what I think I did?
- [ ] Verification gate run with non-zero exit codes captured (see rule 4)?
- [ ] **`gh run list --limit 4` — is CI green on the target branch?** (Rule 10.) If red, the first work item is turning it green. Local-only green is never a "done" claim.
- [ ] Every doc claim that says "passes"/"verified"/"green" cites a commit hash or a literal command output in this session?
- [ ] No fabricated numbers — every "was X / now Y" has a citation or has been rewritten to "first audit" / "no baseline"?
- [ ] TODO_LIST updated for anything completed or partially completed this session?
- [ ] **Did I ship a release?** If yes: did the user explicitly approve the release scope? Never ship breaking changes without explicit approval. Never ship two releases in the same day without a soak period.
- [ ] **Before tagging a release: did `gh run list --limit 4` show the latest CI + Nix runs on the target branch as `success`?** (Rule 9.) A local-only green is not a release-ready green.
- [ ] **Did I draft the GitHub release notes BEFORE pushing the tag?** A tag-without-release window (even 2 minutes) breaks link checkers and confuses downstream consumers.

If any of these cannot be checked, the closing summary must say so explicitly. "Working tree clean" without `git status` in the same response is a process failure, not a shorthand.
