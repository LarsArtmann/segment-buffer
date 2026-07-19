# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **CI hang**: `stress_8_writers_2_readers_throughput` and
  `concurrency_4_writers_1_reader_10k_events` used `FlushPolicy::Batch(4)`,
  creating 20 000 and 2 500 segment files respectively. Under parallel test
  execution this caused pathological I/O that hung CI for hours. Both now use
  `FlushPolicy::Manual` — items stay in-memory during the concurrent phase,
  testing mutex contention instead of the filesystem.
- **CI compile failure**: The README encryption example doctest referenced
  `AesGcmCipher` but `cargo test` (default features) does not enable the
  `encryption` feature. The doctest is now `#[cfg(feature = "encryption")]`-gated
  via the hidden-`fn main()` pattern so it compiles under both feature sets.
- **Nix CI**: `cachix/cachix-action` failed because the binary cache does not
  exist. Added `continue-on-error: true` so builds proceed without caching.

### Added
- **`#[track_caller]`** on `assert_not_reentered` and all 9 public methods that
  call it — re-entrancy panics now point to the user's callback code instead of
  the internal guard function.

## [0.4.2] - 2026-07-19

The "process debt + semver-leak closure" release. All changes are additive or
internal (no breaking changes; drop-in upgrade from v0.4.1). Driven by the
brutally honest v0.4.1 self-review, which uncovered four critical gaps
(`fuzz_hooks` semver leak, no CI loom job, missing dual audit+deny gate,
missing domain-language docs).

### Added

- **`fuzz` Cargo feature** — opt-in feature exposing the `fuzz_hooks` module
  (`parse_filename`, `wrap_envelope`, `unwrap_envelope`, `SegmentRange`) for
  out-of-tree fuzz targets. **Items reachable through this feature are not
  part of the semver contract** and may change in any release without a
  major bump. The in-tree fuzz crate enables this feature in `fuzz/Cargo.toml`.
- **CI `loom` job** — `RUSTFLAGS="--cfg loom" cargo test --features loom
  --release --test loom` runs on every push and PR. The `#![cfg(loom)]` test
  file is invisible to `cargo test` by default and rotted silently between
  v0.4.0 and v0.4.1 (the v0.4.0 `FlushPolicy` change removed fields the loom
  test still referenced); this job prevents that class of regression.
- **Fuzz target: `fuzz_append_all`** — fuzzes `append_all` over arbitrary
  iterator behavior (empty, single, large) with 4 invariants: never panics,
  `pending_count` grows by batch size, `last_seq` advances correctly,
  follow-up empty iterator is a no-op. 771k+ runs / 16s, zero crashes.
- **Property test: `append_all_assigns_contiguous_sequences_across_batches`**
  — varies batch sizes (0–50) across multiple `append_all` calls and asserts
  seqs stay contiguous at the batch boundary. Catches off-by-one regressions
  in the `next_seq` counter.
- **Property test: `sync_disk_bytes_matches_actual_disk_usage`** — after
  every mutation cycle (flush + append), `sync_disk_bytes()` must bring
  `stats().approx_disk_bytes` into exact agreement with the sum of `.zst`
  file sizes on disk. Catches reconciliation drift.
- **`docs/DOMAIN_LANGUAGE.md`** — glossary for segment, head_seq, next_seq,
  acked_seq, envelope, flush, recover. Codifies the ubiquitous vocabulary
  for issues, doc comments, and commit messages.
- **`docs/CIPHERS.md`** — cipher internals + worked bring-your-own-AEAD
  examples: ChaCha20-Poly1305 (via the `chacha20poly1305` crate), no-op
  cipher (testing only), and an explanation of what the cipher does and
  does not see (item boundaries, filename, envelope).
- **`docs/perf/2026-07-19_v0.4.1_stress_throughput.md`** — v0.4.1 stress
  test baseline: ~397k events/sec under 8-writer × 2-reader contention with
  `FlushPolicy::Manual`. Reproduction command + interpretation included.

### Changed

- **`fuzz_hooks` is now `#[cfg(any(test, feature = "fuzz"))]`** instead of
  `#[doc(hidden)] pub`. **This closes a v0.4.1-introduced semver leak.**
  `#[doc(hidden)]` hides items from rustdoc but does NOT remove them from
  the public API surface; the cfg gate does both. See `CONTRIBUTING.md` →
  "Internal hooks: `#[cfg]` over `#[doc(hidden)]`" for the rationale.
- **`Cargo.toml` description** rewritten for crates.io search clarity:
  "Durable bounded queue: batch-spills to zstd+CBOR segment files with
  ack-based deletion and filename-based crash recovery. No WAL, no metadata
  db."
- **CI `supply-chain` job** renamed to `cargo audit + cargo deny` for
  discoverability (no behavior change).

### Fixed

- **Broken `AesGcmCipher` doc link warning** under default features (the
  `[AesGcmCipher]` intradoc link failed to resolve when the `encryption`
  feature was off). Replaced with a prose reference to "the `AesGcmCipher`
  behind the `encryption` feature". `RUSTDOCFLAGS="-D warnings" cargo doc`
  is now clean under all feature combinations.

### Internal

- **AGENTS.md verification discipline** gained two new hard rules:
  - Rule 5: "The supply-chain gate is BOTH `cargo audit` AND `cargo deny
    check`." They pull from different advisory sources in edge cases.
  - Rule 6: "The loom gate is `RUSTFLAGS='--cfg loom' cargo test --features
    loom --test loom --release`." `#![cfg(loom)]` files are invisible to
    default `cargo test` and silently rot.
- **CONTRIBUTING.md** gained a new section: "Internal hooks: `#[cfg]` over
  `#[doc(hidden)]`" — codifies the lesson from the v0.4.1 semver leak so
  the next agent doesn't repeat it.

## [0.4.1] - 2026-07-19

The "safety + trust depth" release. All changes are additive (no breaking
changes). On-disk format, encryption contract, and API shapes are unchanged
from v0.4.0.

### Added

- **`for_each_from` re-entrancy guard** — calling any `&self` method on the
  buffer from inside a `for_each_from` callback now panics with a clear message
  (`{method}: cannot call from within a for_each_from callback`) instead of
  silently deadlocking. The guard is Drop-cleared for panic safety, so a
  panicking callback does not brick the buffer. (Closes the v0.4.0 footgun.)
- **`append_all<I: IntoIterator<Item = T>>`** — batch append under a single
  lock acquisition. Returns the last assigned sequence number. The whole batch
  gets contiguous seqs atomically; flush is checked once at the end. Bench:
  `benches/bench_append_all.rs` quantifies the lock-acquisition saving vs a
  loop of `append`.
- **`SegmentBuffer::path()`** — returns `&Path` to the segment directory.
  Removes the need to `Debug`-parse the buffer to reach the directory.
- **`SegmentBuffer::config()`** — returns `&SegmentConfig` the buffer was
  opened with. Lets callers inspect the flush policy, compression level, and
  cipher presence without re-deriving them.
- **`SegmentBuffer::sync_disk_bytes()`** — re-stats the segment directory and
  stores the authoritative total. Corrects drift when an external process
  (backup, compaction, manual cleanup) touches the directory.
- **`fuzz_hooks` module** (`#[doc(hidden)]`) — exposes `parse_filename`,
  `unwrap_envelope`, `wrap_envelope`, and `SegmentRange` for fuzz targets.
  Not part of the public API.
- **Two new fuzz targets**: `fuzz_parse_filename` (17M+ runs / 16s, zero
  crashes) and `fuzz_envelope` (15M+ runs / 16s, zero crashes; fuzzer
  discovered the `SBF1` magic dictionary entry organically).
- **Property tests**: `FlushPolicy::Manual` never auto-flushes (up to 499
  appends); `read_from(start, limit)` ⊆ `read_from(start, larger_limit)`;
  `delete_acked` pending_count is monotone non-increasing; `for_each_from`
  visits the same items as `read_from`.
- **Throughput stress test**: 8 writers × 2 readers × 80k events, reports
  events/sec under contention.
- **Loom test**: `append_all` batch atomicity under concurrent `append`.
- **CI workflows**: nightly cargo-fuzz (`fuzz.yml`), weekly flake.lock update
  (`update-flake-lock.yml`), cargo-audit + cargo-deny supply-chain job,
  dependabot.yml for GitHub Actions + cargo.
- **Docs**: `docs/PERFORMANCE.md` (methodology), `docs/RELEASE.md` (runbook),
  `docs/MSRV.md` (policy).

### Changed

- **`packages.default` in `flake.nix`** now builds with Rust 1.85 (the declared
  MSRV) via `craneLibMsrv`, proving the package builds on its floor — not just
  on whatever nixpkgs stable ships.
- **`dtolnay/rust-toolchain`** pinned to a commit hash in all CI workflows
  (supply-chain hygiene).
- **`nix.yml` cachix-action** guarded to the canonical repo + optional token,
  so forks don't attempt uploads to a cache that doesn't exist.
- **README perf paragraph** now carries the methodology caveat inline ("single-
  run, single-machine; see docs/PERFORMANCE.md").
- **README comparison table** now carries a freshness disclaimer.
- **AGENTS.md session-end checklist** gains "release scope approval" and
  "draft release notes before tagging" items (process guard against the
  v0.4.0 failure).

### Fixed

- **Loom test was broken since v0.4.0** — referenced removed `max_batch_events`
  / `flush_interval_secs` fields and had an inner attribute inside a function
  body. Now uses `FlushPolicy::Manual` via the builder API. The breakage was
  invisible because `#![cfg(loom)]` skips compilation unless `--cfg loom` is set.

### Internal

- `SegmentRange` fields are now `pub` (were `pub(crate)`) to support the
  `fuzz_hooks` re-export. The `segment` module itself stays private; the fields
  are only reachable through `#[doc(hidden)] fuzz_hooks`.

## [0.4.0] - 2026-07-19

The "API ergonomic + perf" release. Breaking because it removes two
`SegmentConfig` fields (`max_batch_events`, `flush_interval` — replaced by
`FlushPolicy`), changes the `SegmentError::Io` variant from tuple to struct,
and renames the now-private `flush_interval_secs` builder method. On-disk
format, encryption contract, and trait shape are unchanged.

### Added

- **`SegmentConfig::builder()`** — fluent builder over `Default + setters`.
  Removes the `Default + field reassignment` workaround every external caller
  had to use under `#[non_exhaustive]`. Convenience setters: `flush_policy`,
  `flush_at_batch_size`, `flush_at_interval`, `flush_at_batch_or_interval`,
  `flush_manually`, `max_size_bytes`, `compression_level`, `cipher`.
- **`FlushPolicy` enum** (`Batch(usize)` / `Interval(Duration)` /
  `BatchOrInterval { batch_size, interval }` / `Manual`). Replaces the silent
  OR-combination of `max_batch_events` + `flush_interval_secs` that callers
  had no way to disable.
- **`RecoveryReport` + `SegmentBuffer::open_with_report()`** — returns
  `(SegmentBuffer<T>, RecoveryReport)` so callers can inspect what recovery
  found (segment count, head/next seq, disk bytes, removed tmp files)
  programmatically. `open()` is unchanged and delegates internally.
- **`for_each_from(start, limit, F)`** lending iterator — the zero-clone
  counterpart to `read_from`. Benched ~21× faster on 1000 in-memory items
  (1.2 µs vs 26 µs). Documented deadlock warning: the closure must not
  re-enter buffer methods while iterating the in-memory tail.
- **`examples/crash_recovery.rs`** — demonstrates that flushed segments
  survive a process restart and unflushed ones do not, plus the new
  `open_with_report` API.
- **`examples/mpmc.rs`** — 4 writers × 1 reader sharing one
  `Arc<SegmentBuffer>`, draining via `read_from + delete_acked`.
- **`.github/workflows/nix.yml`** — CI workflow running
  `nix flake check`, `nix build .#default`, the test check, and treefmt.
- **`deny.toml`** — cargo-deny config (advisories, licenses, bans, sources).
  All four pass green as of release.
- **`renovate.json`** — weekly dependency updates, with `nix` and
  `github-actions` enabled alongside `cargo`.
- **`release.toml`** — cargo-release config (no auto-push; tags via
  `sign-tag`, hand-curated GitHub releases via `gh`).
- **Display snapshot test for `SegmentError::Io` with `path: Some(...)`**
  and a test for `with_path` (the path-attach helper).

### Changed

- **`SegmentConfig` lost `max_batch_events` and `flush_interval`**, replaced
  by a single `flush_policy: FlushPolicy` field. Migration:
  ```rust
  // before
  SegmentConfig { max_batch_events: 256, flush_interval_secs: 5, ..Default::default() }
  // after
  SegmentConfig::builder()
      .flush_at_batch_or_interval(256, Duration::from_secs(5))
      .build()
  ```
- **`SegmentError::Io` is now a struct variant**:
  `Io { path: Option<PathBuf>, source: std::io::Error }`. The bare
  `From<io::Error>` impl preserves `?` ergonomics with `path: None`; the
  `with_path` helper and direct construction attach path context at
  high-value call sites (`write_segment`, `read_segment`, `scan_segments`).
  Display: `"I/O error: {source}"` when `path` is `None`, or
  `"I/O error for {path}: {source}"` when set.
- **`approx_disk_bytes` is now `AtomicU64`** outside `BufferInner`.
  `flush()` no longer re-acquires the mutex just to bump one `u64`;
  `store_pressure()` loads the atomic without locking at all.
- **`scan_segments()` results are cached** — invalidated by `flush`,
  `delete_acked`, `recover`. `read_from` followed by `delete_acked` no
  longer pays the directory-scan cost twice.
- **Tracing fields standardized** — every event now carries `path`, `seq`,
  and `bytes` where they make sense, replacing the inconsistent
  `head_seq` / `next_seq` / `disk_bytes` / `start` / `end` mix.

### Fixed

- The pre-v0.4.0 `SegmentError::Io` variant dropped path context. Operators
  saw `"I/O error: ..."` with no file. Now the offending path is carried
  whenever it is in scope.

## [0.3.0] - 2026-07-19

This release closes the v0.2.0 semver/honesty debt identified in the
post-v0.2.0 self-reviews. It is **a breaking release** because
`BufferStats` and `SegmentConfig` are now `#[non_exhaustive]` — downstream
code that uses struct literals to construct either type must switch to
`Default::default()` + field reassignment (or, in v0.4.0, the planned
`SegmentConfig::builder()`). The break is intentional and minor:
the on-disk format, the trait shape, the error types, and the encryption
contract are all unchanged from v0.2.0.

### Added

- **`Debug` impl for `SegmentBuffer<T>`** — mirrors the `BufferStats` field
  set plus the directory path. Does NOT print in-memory `unflushed` items,
  so `T: Debug` is not required. Snapshot test in `src/tests.rs`.
- **`CipherError::with_source` doc-test** — the `source()`-chaining
  constructor now has a runnable example in its rustdoc.
- **Display snapshot tests for every `SegmentError` variant and both
  `CipherError` constructors** (`msg` + `with_source`) — locks the
  operator-facing format strings so a careless `thiserror`-attribute edit
  shows up as a test failure rather than silently shifting log output.
- **`benches/bench_stats.rs`** — criterion micro-bench comparing `stats()`
  (single lock + 7-field snapshot, ~12 ns) to three individual accessors
  (~31 ns). The "cheaper" doc claim now cites measured numbers.
- **`docs/perf/2026-07-19_v0.1.0-vs-v0.2.0.md`** — controlled baseline
  captured via `git worktree v0.1.0 vs HEAD`: append 30–65% slower on small
  batches (envelope + stats bookkeeping has a per-write cost), recover
  40–45% faster (recovery refactor paid off). README Status section cites
  this and the trade-off is honest.
- **Verification discipline section in `AGENTS.md`** — four hard rules and
  a session-end checklist, installed after three same-day sessions produced
  self-reviews that claimed success without running the verification gate,
  fabricated working-tree state, and invented baselines.
- **rust-overlay integration in `flake.nix`** with two new devShells:
  `nix develop .#msrv` (pinned Rust 1.85.0) and `nix develop .#fuzz`
  (nightly for `cargo-fuzz`). All three MSRV checks (`cargo check`,
  `cargo test`, `cargo clippy -- -D warnings`) now run locally on the
  declared MSRV; both fuzz targets now run locally for ≥60s each.

### Changed

- **`BufferStats` and `SegmentConfig` are now `#[non_exhaustive]`** —
  paying down the v0.2.0-introduced semver debt. Downstream struct-literal
  construction must switch to `Default::default()` + field reassignment.
  In-crate construction (tests, examples, benches) is unaffected. This is
  the breaking change that motivates cutting 0.3.0.

### Fixed

- **Corrected the "auto-staging Crush hook" myth** in the 03-14 and 04-22
  self-reviews. Investigation during the v0.3.0 planning session found no
  such hook exists; the only Crush hook is `commit-diff-context.sh`, which
  fires when a commit runs (to inject diff context) and does not stage or
  commit. The three sessions' "lost track of working-tree state" was a real
  pattern, but the _attribution_ was wrong — the cause was the assistant
  not running `git status`/`git log` before claiming state. Now codified as
  Verification discipline rule 1 in `AGENTS.md`.
- **`PROPTEST_CASES=256` pinned in CI** — removes a flaky-machine variable
  and matches the release-build default explicitly.

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

[Unreleased]: https://github.com/LarsArtmann/segment-buffer/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.4.2
[0.4.1]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.4.1
[0.4.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.4.0
[0.3.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.3.0
[0.2.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.2.0
[0.1.0]: https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.1.0
