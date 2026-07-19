# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (move to CHANGELOG).

---

## v0.4.2 follow-ups (shipping in this release)

Process debt + semver-leak fix + CI hardening uncovered by the v0.4.1 self-review.
No breaking changes; drop-in upgrade from v0.4.1.

- [x] **Gate `fuzz_hooks` behind `#[cfg(any(test, feature = "fuzz"))]`** instead of `#[doc(hidden)] pub` — closes the v0.4.1 semver leak.
- [x] **Add `fuzz` Cargo feature** — opt-in feature for unstable internals.
- [x] **Add CI `loom` job** — `#![cfg(loom)]` test file was invisible to CI and rotted silently between v0.4.0 and v0.4.1.
- [x] **Document dual `cargo audit` + `cargo deny` verification gate** in AGENTS.md.
- [x] **Document `#[cfg]` over `#[doc(hidden)]`** in CONTRIBUTING.md.
- [x] **Add `docs/DOMAIN_LANGUAGE.md`** — glossary for segment, head_seq, next_seq, acked_seq, envelope, flush, recover.
- [x] **Add `docs/CIPHERS.md`** — bring-your-own AEAD (ChaCha20-Poly1305, no-op) worked examples.
- [x] **Copywriting pass on `Cargo.toml` `description`** — punchier one-liner for crates.io search.
- [x] **Property test: `append_all` contiguous seqs across batches** — varying batch sizes, including empty batches.
- [x] **Property test: `sync_disk_bytes` matches actual du** — after every mutation cycle.
- [x] **Fuzz target: `fuzz_append_all`** — iterator behavior (empty, single, large) with 4 invariants.
- [x] **Stress test throughput baseline** captured in `docs/perf/2026-07-19_v0.4.1_stress_throughput.md` (~397k events/sec under 8-writer contention).
- [x] **Fix broken `AesGcmCipher` doc link** warning under default features.

---

## v0.4.1 (shipped — kept for reference)

Additive API + safety fixes for v0.4.0. No breaking changes.

- [x] **Make `for_each_from` re-entrancy-safe** — `AtomicBool` guard panics with a clear message on callback re-entry instead of silent deadlock. Drop-cleared for panic safety.
- [x] **Property test for `FlushPolicy::Manual`** — asserts no auto-flush across up to 499 appends, then verifies explicit flush still works.
- [x] **Add `cargo audit` + `cargo deny` to CI** — dedicated `supply-chain` job.
- [x] **Add `RecoveryReport` doc-test** showing recovery over a populated directory.
- [x] **Add inline caveats to the README perf paragraph** — methodology caveat is now in the same paragraph as the claim.
- [x] **Pin `packages.default` in `flake.nix` to Rust 1.85** — `craneLibMsrv` proves the package builds on the declared MSRV.
- [x] **Add `SegmentBuffer::path()` and `config()` accessors** — no more `Debug`-parsing to reach the dir/config.
- [x] **Add `sync_disk_bytes()`** — re-stats the directory to correct external-manipulation drift.
- [x] **Add `append_all<I: IntoIterator<Item = T>>`** — single-lock batch primitive.
- [x] **Property test: `read_from` limit monotonicity** — `read_from(start, limit)` is a prefix of `read_from(start, larger_limit)`.
- [x] **Property test: `delete_acked` monotone non-increasing `pending_count`** — acking more never adds items.
- [x] **Property test: `for_each_from` ↔ `read_from` equivalence** — same items, same order, same seqs.
- [x] **Bench: `delete_acked` at 10k segments** — scale test that monitor365 actually hits.
- [x] **Bench: `append_all` vs loop `append`** — quantifies the batch-lock saving.
- [x] **Pin `dtolnay/rust-toolchain` to commit hash** — supply-chain hygiene.
- [x] **Add `dependabot.yml`** — belt-and-braces alongside Renovate.
- [x] **Fix `nix.yml` cachix** — guarded to canonical repo + optional token.
- [x] **Add cargo-fuzz scheduled workflow** — nightly 5-min fuzz runs.
- [x] **Add flake.lock update workflow** — weekly auto-PR.
- [x] **Add `#[cfg(doctest)]` harness** for examples as doc-tests.
- [x] **Add release-scope-approval to AGENTS.md checklist** — process guard against the v0.4.0 failure.
- [x] **Create `docs/PERFORMANCE.md`** — methodology + how to reproduce + noise interpretation.
- [x] **Create `docs/RELEASE.md`** — cut-a-release runbook.
- [x] **Create `docs/MSRV.md`** — MSRV policy + verification.

---

## v0.4.0 (shipped — kept for reference)

These breaking API improvements were batched into v0.4.0 so users upgrade once.

- [x] **`SegmentConfig::builder()`** with defaults — fluent builder for the `#[non_exhaustive]` struct.
- [x] **`FlushPolicy` enum** — `Batch` / `Interval` / `BatchOrInterval` / `Manual`; replaces the silent-combine of two fields.
- [x] **`RecoveryReport` returned from `open_with_report()`** — segments found, bytes, head/next seq, removed tmp count.
- [x] **Typed `SegmentError::Io { path, source }`** — struct variant with `Option<PathBuf>`; `with_path()` helper; `From<io::Error>` preserved.
- [x] **`for_each_from` lending iterator** — zero-clone in-memory path, ~21× faster than `read_from`.
- [x] **`AtomicU64` for `approx_disk_bytes`** — `flush()` no longer re-acquires the mutex.
- [x] **Cache `scan_segments()`** — invalidated by every on-disk mutation.
- [x] **`#[non_exhaustive]` on `BufferStats` + `SegmentConfig`** — semver debt closed.
- [x] **`Debug` impl for `SegmentBuffer<T>`** — mirrors `BufferStats` + dir.
- [x] **`SegmentCipher → SegmentAead` rename** — DECISION: REJECT. Trait contract is not strictly AEAD; documented.
- [x] **`crash_recovery` + `mpmc` examples** — demonstrate the durability + MPMC contracts.

---

## v0.5.0 candidates (next breaking batch)

Deferred breaking changes — batch them so users upgrade once.

- [ ] **`Arc<dyn SegmentCipher>` instead of `Box`** — so `SegmentConfig` can be `Clone`. Today the `Box` makes the config non-`Clone`, which surprises callers who expect to inspect/reuse it.
- [ ] **`SegmentIter<'_, T>` lending iterator type** — return an actual GAT-based iterator from `for_each_from` instead of taking a closure, for true iterator ergonomics (`for (seq, item) in buf.iter_from(0)?`).
- [ ] **`IoSite` enum for `SegmentError::Io`** — replace `Option<PathBuf>` with `IoSite::Dir | IoSite::Segment(PathBuf) | IoSite::Unknown` to make the "no path" case explicit.
- [ ] **`TryClone` story for `SegmentConfigBuilder`** — once `.cipher(Box::new(...))` is called, the builder is non-`Clone`. Either document loudly or provide a `TryClone` that errors on cipher-bearing configs.
- [ ] **mtime probe for scan cache** — cheap `stat` to validate the cache against external directory manipulation (today the cache is invalidated only by in-process mutations).

## Concurrency & provability

- [x] **Loom test for `append` + `stats()`** — 2 tests in `tests/loom.rs` covering the in-memory path.
- [x] **CI `loom` job** — added v0.4.2; runs `RUSTFLAGS="--cfg loom" cargo test --features loom --release --test loom` so the file cannot rot silently again.
- [ ] **Loom test for `delete_acked` + `append` interleaving** — requires abstracting I/O behind a trait loom can mock; real engineering work.
- [ ] **`#[track_caller]`** on panicking paths (defensive — the re-entrancy guard is the only panic today).
- [ ] **Consider `RwLock` for read-heavy workloads** — `read_from` is read-only; `append`/`flush`/`delete_acked` write. Measure first.
- [x] **Stress test: 8 writers × 2 readers × 80k events with throughput reporting** — added v0.4.1; baseline captured v0.4.2. Fixed v0.4.3: switched to `FlushPolicy::Manual` to avoid creating 20 000 segment files that hung CI.
- [ ] **Stress test: 16 writers × 4 readers × 1M events with p50/p99 latency histogram** — today's stress test reports throughput only, not latency distribution.

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

- [x] **`for_each_from` lending iterator** — zero-clone in-memory reads (done in v0.4.0).
- [x] **Atomic `approx_disk_bytes`** — done in v0.4.0.
- [x] **Cache `scan_segments()`** — done in v0.4.0.
- [ ] **Profile-guided optimization of the hot path** — criterion benches exist but have not been profiled with `cargo flamegraph`.
- [ ] **Consider `SmallVec<[T; 16]>` for `unflushed`** — avoid the initial heap allocation for small batches (adds a dep).
- [ ] **Bench `read_from` after the scan cache landed** — the v0.1.0-vs-v0.2.0 numbers predate the cache.

## Docs & polish

- [x] **`docs/DOMAIN_LANGUAGE.md`** — glossary for segment, head_seq, next_seq, acked_seq, envelope, flush, recover. Added v0.4.2.
- [x] **`docs/CIPHERS.md`** — AES-GCM internals + ChaCha20-Poly1305 + no-op cipher worked examples. Added v0.4.2.
- [x] **Copywriting pass** on `Cargo.toml` `description`. Done v0.4.2.
- [ ] **Skill-contract debt** — produce the HTML artifacts required by the `code-quality-scan`, `architecture-review`, `full-code-review`, and `nix-flake-migration` skills (or explicitly renegotiate them).

## CI / tooling

- [x] **`cargo +nightly fuzz` in CI** — nightly scheduled workflow added v0.4.1.
- [x] **`PROPTEST_CASES=256` pin in CI** — done.
- [x] **Nix fuzz app** (`apps.fuzz`) — done.
- [x] **cargo-deny config** — done v0.4.0.
- [x] **Renovate + Dependabot** — done v0.4.1.
- [x] **cargo-release config** — done v0.4.0.
- [x] **Nix CI workflow** — done v0.4.0.
- [x] **MSRV pin in flake** — done v0.4.1 (packages.default now uses 1.85).
- [x] **CI `loom` job** — done v0.4.2.
- [ ] **macOS flake verification** (`aarch64-darwin`, `x86_64-darwin`) — flake check only runs on x86_64-linux today.
- [ ] **Sign commits** — `sign-commit = true` is set in `release.toml` and `commit.gpgsign = true` in git config, but SSH signing fails: `gpg.ssh.allowedSignersFile` is not configured. Tags are signed; regular commits are not.
- [x] **Add publish-on-tag workflow** triggered by `v*` tags — done v0.4.2 (`.github/workflows/publish.yml`). Dormant until `CARGO_REGISTRY_TOKEN` secret is set.

## Investigation

- [ ] **Tighten `T: 'static`** — investigate whether it can be relaxed (needed for the mutex, but worth confirming).
- [ ] **Extract AES-GCM cipher into its own feature/crate boundary** for users who want only the trait.
- [ ] **Profile the hermetic Nix build** (~164s for test check; most is zstd-sys compiling bundled C). Could pre-build zstd as a Nix dependency via `ZSTD_SYS_USE_PKG_CONFIG=1`.
- [ ] **Profile the append hot path with `cargo flamegraph`** — the v0.1.0-vs-v0.2.0 30–65% regression has never been profiled.
- [x] **`cargo publish --dry-run`** — verified; real `cargo publish` executed for v0.4.1.

## Crates.io publishing

- [x] **crates.io API token configured** locally; works for manual `cargo publish`.
- [ ] **Set up a crates.io API token** in GitHub Actions secrets for automated publishing on tag (`CARGO_REGISTRY_TOKEN` — the `publish.yml` workflow is dormant without it).
