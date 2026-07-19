# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (move to CHANGELOG).

---

## v0.2.0 follow-ups (P0 debt from the v0.2.0 sweep)

Items the v0.2.0 self-review (`docs/status/2026-07-19_04-22_*`) flagged as
under-delivered or skipped. These do not require a release; do them on `HEAD`.

- [ ] **Run `cargo +1.85 check --all-targets --features encryption` locally.** The `ErrorExt` trait upcast (src/cipher.rs) and the `const fn assert_send_sync` assertion (src/lib.rs) are written to preserve MSRV 1.85 but have never been verified outside CI. Install 1.85 via Nix (`nix shell github:oxalica/rust-overlay#rust-bin.stable."1.85.0".default`) if rustup is unavailable.
- [ ] **Run real `cargo +nightly fuzz`** for ≥60s per target. Proptest analogues are interim mitigation only. Install nightly via Nix if rustup is unavailable.
- [ ] **Doc-test for `CipherError::with_source`** — show wrapping an AEAD error and reading it back via `source()`.
- [ ] **`Debug` impl for `SegmentBuffer<T>`** — standard collection ergonomics; current absence makes `dbg!()`-style debugging worse.
- [ ] **Snapshot/golden tests for `SegmentError` and `CipherError` Display** — lock in the format strings before they ossify.
- [ ] **Bench `stats()` vs individual accessors** — prove or remove the "cheaper and more consistent" claim in the doc comment.
- [ ] **Controlled benchmark baseline** — checkout v0.1.0 (pre-envelope), capture criterion baseline, compare to HEAD. Or update README/CHANGELOG to honestly say "no controlled baseline".

---

## v0.3.0 (next breaking release)

These are breaking changes; batch them so users upgrade once.

- [ ] **`SegmentConfig::builder()`** with defaults — replace the public-field struct with a builder so new fields don't break callers.
- [ ] **`flush_interval: Duration`** instead of `flush_interval_secs: u64` — idiomatic, composable.
- [ ] **`RecoveryReport` returned from `open()`** — segments found, bytes, head/next seq, time spent. Today this is logged but not returned.
- [ ] **`FlushPolicy` enum** (Batch / Interval / Manual) to replace the two `SegmentConfig` fields that silently combine.
- [ ] **Typed `SegmentError::Io`** — currently bare `#[from] io::Error` drops path context.
- [ ] **Consider `SegmentCipher` → `SegmentAead` rename** — the trait is specifically AEAD-shaped (self-describing nonce-in-band); the name lies slightly.
- [ ] **`#[non_exhaustive]` on `BufferStats`** — v0.2.0 added the struct with all-public fields; adding a field later is currently breaking.
- [ ] **`#[non_exhaustive]` on `SegmentConfig`** — older debt of the same class.

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
