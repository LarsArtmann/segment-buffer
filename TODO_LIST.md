# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (move to CHANGELOG).

---

## v0.2.0 follow-ups (P0 debt from the v0.2.0 sweep)

Items the v0.2.0 self-review (`docs/status/2026-07-19_04-22_*`) flagged as
under-delivered or skipped. These do not require a release; do them on `HEAD`.

- [x] **Run `cargo +1.85 check --all-targets --features encryption` locally.** Verified 2026-07-19 via Nix: `devShells.msrv` (rust-overlay 1.85.0) ran `cargo check`, `cargo test --no-fail-fast`, and `cargo clippy -- -D warnings` all green. The MSRV claim is now backed by local evidence, not just CI.
- [x] **Run real `cargo +nightly fuzz`** for ≥60s per target. Verified 2026-07-19 via Nix `devShells.fuzz`: both targets ran for 60s with zero crashes. `fuzz_corrupted_read`: 187,811 runs, 392 coverage blocks. `fuzz_recovery` numbers in commit history. `libfuzzer-sys` recovered the `SBF1` magic dict entry organically — envelope path is exercised.
- [x] **Doc-test for `CipherError::with_source`** — added in `src/cipher.rs` showing an `AeadError` wrapped and read back via `source()`.
- [x] **`Debug` impl for `SegmentBuffer<T>`** — added, mirrors `BufferStats` field set + dir; does not print in-memory items so `T: Debug` is not required. Snapshot test in `src/tests.rs::debug_impl_formats_cleanly`.
- [x] **Snapshot/golden tests for `SegmentError` and `CipherError` Display** — added in `src/tests.rs` covering all 4 `SegmentError` variants and both `CipherError` constructors (`msg` + `with_source`).
- [x] **Bench `stats()` vs individual accessors** — `benches/bench_stats.rs` measures `stats()` ≈ 12 ns vs 3 individual accessors ≈ 31 ns. The "cheaper" claim in `stats()`'s doc comment now cites those numbers in a Performance section.
- [x] **Controlled benchmark baseline** — captured 2026-07-19 via `git worktree` v0.1.0 vs HEAD. Results in [`docs/perf/2026-07-19_v0.1.0-vs-v0.2.0.md`](docs/perf/2026-07-19_v0.1.0-vs-v0.2.0.md). Honest summary: append 30–65% slower on small batches (envelope cost), recover 40–45% faster (recovery refactor paid off). Cross-link added to README.md Status section.

---

## v0.4.0 (next breaking release)

These breaking API improvements were originally floated for v0.3.0 but deferred
to v0.4.0 so that v0.3.0 could be a minimal "close the v0.2.0 debt" release.
They are batched so users upgrade once. Design principle: each one must be
prototyped + benched before adoption; reject the ones that don't pay rent.

- [ ] **`SegmentConfig::builder()`** with defaults — replace the public-field struct with a builder so new fields don't break callers. Now that the struct is `#[non_exhaustive]` (v0.3.0), the builder is ergonomic debt, not semver debt — but downstream code is paying for it today via the `Default + field reassignment` workaround.
- [ ] **`flush_interval: Duration`** instead of `flush_interval_secs: u64` — idiomatic, composable, plays well with `humantime`.
- [ ] **`RecoveryReport` returned from `open()`** — segments found, bytes, head/next seq, time spent. Today this is logged but not returned.
- [ ] **`FlushPolicy` enum** (Batch / Interval / Manual) to replace the two `SegmentConfig` fields that silently combine.
- [ ] **Typed `SegmentError::Io`** — currently bare `#[from] io::Error` drops path context.
- [ ] **Consider `SegmentCipher` → `SegmentAead` rename** — the trait is specifically AEAD-shaped (self-describing nonce-in-band); the name lies slightly. Decision task: rename, or document the AEAD-only intent and keep the name.
- [x] **`#[non_exhaustive]` on `BufferStats`** — added in v0.3.0; `SegmentConfig` too. Construction-by-literal is no longer part of the public API; downstream consumers use `Default + field reassignment` or (future) the `SegmentConfig::builder()` once v0.4.0 lands.
- [x] **`#[non_exhaustive]` on `SegmentConfig`** — same class of debt, closed in the same v0.3.0 batch.

## Concurrency & provability

- [ ] **Loom test** for `append` / `flush` / `delete_acked` — exhaustive schedule check, not just the single-schedule stress test we have today.
- [ ] **`#[track_caller]`** on panicking paths (defensive — none today).
- [ ] **Consider `RwLock` for read-heavy workloads** — `read_from` is read-only; `append`/`flush`/`delete_acked` write. Measure first.

## Format & storage

- [ ] **Per-segment Blake3 checksum** in the reserved envelope bytes (bit-rot detection distinct from cipher auth failures).
- [ ] **Envelope v2 design doc** — sketch the migration path for when v2 lands.
- [ ] **Compression-algorithm negotiation** via reserved byte (zstd, lz4, none).
- [ ] **Metadata block in envelope** (item count, byte count, schema hash).
- [ ] **`SegmentStore` trait** abstraction (local FS, S3, in-memory) — defer until second impl exists.
- [ ] **Async I/O feature** (tokio) — preserve "mutex never held across I/O" invariant under cancellation.
- [ ] **ChaCha20-Poly1305 cipher** under a feature flag.
- [ ] **XChaCha20-Poly1305** for extended nonces (no 2^32 message limit per key).

## Performance

- [ ] **`read_from` clones every event** — quantify with a bench, consider a `for_each_from(start, limit, F)` lending iterator (zero-clone reads).
- [ ] **Atomic `approx_disk_bytes`** — `flush()` re-acquires the lock just to bump one `u64`; an `AtomicU64` would remove the second lock.
- [ ] **Cache `scan_segments()`** — re-reads the directory on every `read_from`/`delete_acked` call.
- [ ] **Profile-guided optimization of the hot path** — criterion benches exist but have not been profiled.

## Observability & ops

- [ ] **`tracing` fields standardization** — every event carries `path`, `seq`, `bytes`.
- [ ] **Crash-recovery example** — runnable `examples/crash_recovery.rs` showing the durability contract.
- [ ] **MPMC example** — runnable `examples/mpmc.rs` showing multiple writers + readers.
- [ ] **cargo-deny config** for license/security advisories.
- [ ] **Renovate/dependabot** config for dependency updates.
- [ ] **cargo-release config** for consistent releases.
- [ ] **Nix CI workflow** (`.github/workflows/nix.yml`) mirroring `nix flake check`.
- [ ] **MSRV pin in flake** (Rust 1.85 overlay) for hermetic MSRV verification.
- [ ] **macOS flake verification** (`aarch64-darwin`, `x86_64-darwin`).

## Docs & polish

- [ ] **`#![doc = include_str!("../README.md")]`** on crate root for docs.rs landing page (needs README code blocks made rustdoc-clean first).
- [ ] **Doc-tests for every public method** (currently 15).
- [ ] **Document semver/stability policy** in CONTRIBUTING or a dedicated `docs/policies.md`.
- [ ] **Copywriting pass** on `Cargo.toml` `description` and CHANGELOG prose quality.
- [ ] **Skill-contract debt** — produce the HTML artifacts required by the `code-quality-scan`, `architecture-review`, `full-code-review`, and `nix-flake-migration` skills (or explicitly renegotiate them).

## CI / tooling

- [ ] **`cargo +nightly fuzz` in CI** as a scheduled job (decision needed: required on every PR, scheduled, or manual only).
- [ ] **`PROPTEST_CASES=256` pin in CI** so proptest doesn't become a flakiness source.
- [ ] **Nix fuzz app** (`apps.fuzz`) for reproducible fuzzing.

## Investigation

- [ ] **Tighten `T: 'static`** — investigate whether it can be relaxed (needed for the mutex, but worth confirming).
- [ ] **Extract AES-GCM cipher into its own feature/crate boundary** for users who want only the trait.
- [ ] **Profile the hermetic Nix build** (~164s for test check; most is zstd-sys compiling bundled C). Could pre-build zstd as a Nix dependency via `ZSTD_SYS_USE_PKG_CONFIG=1`.
