# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (move to CHANGELOG).

---

## v0.4.1 follow-ups (shipping in this release)

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
- [ ] **Loom test for `delete_acked` + `append` interleaving** — requires abstracting I/O behind a trait loom can mock; real engineering work.
- [ ] **`#[track_caller]`** on panicking paths (defensive — the re-entrancy guard is the only panic today).
- [ ] **Consider `RwLock` for read-heavy workloads** — `read_from` is read-only; `append`/`flush`/`delete_acked` write. Measure first.
- [ ] **Stress test: 8 writers × 2 readers × 100k events with latency histogram** — today's test is 4×1×10k, which proves correctness, not performance under contention.

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

- [ ] **`docs/DOMAIN_LANGUAGE.md`** — glossary for segment, head_seq, next_seq, acked_seq, envelope, flush, recover.
- [ ] **Copywriting pass** on `Cargo.toml` `description` and CHANGELOG prose quality.
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
- [ ] **macOS flake verification** (`aarch64-darwin`, `x86_64-darwin`) — flake check only runs on x86_64-linux today.
- [ ] **Sign commits** — tags are signed; commits are not. Add `sign-commit = true` to git config.

## Investigation

- [ ] **Tighten `T: 'static`** — investigate whether it can be relaxed (needed for the mutex, but worth confirming).
- [ ] **Extract AES-GCM cipher into its own feature/crate boundary** for users who want only the trait.
- [ ] **Profile the hermetic Nix build** (~164s for test check; most is zstd-sys compiling bundled C). Could pre-build zstd as a Nix dependency via `ZSTD_SYS_USE_PKG_CONFIG=1`.
- [ ] **`cargo publish --dry-run`** — verify the package has no packaging issues before the real publish.

## Crates.io publishing

- [ ] **Set up a crates.io API token** in GitHub Actions secrets for automated publishing on tag.
- [ ] **Add publish-on-tag workflow** triggered by `v*` tags.
