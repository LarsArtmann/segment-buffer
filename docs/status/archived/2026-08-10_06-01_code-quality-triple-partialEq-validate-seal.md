# Status: Code Quality Triple — PartialEq, validate(), Sealed Trait

> **FULLY RESOLVED** — all work shipped. Forward-looking items harvested into
> `TODO_LIST.md` on 2026-08-10. Archived.

**Date:** 2026-08-10 06:01
**Session scope:** Three "Code quality" TODO items completed, plus human-readable BufferStats bytes
**Branch:** `master` (7 files modified, uncommitted — auto-git daemon will commit)

---

## What this session did

The user asked to finish the three "Code quality" items from TODO_LIST.md:

1. Derive/implement `PartialEq` for `SegmentConfig`
2. Add `FlushPolicy::validate()` method
3. Seal the `SegmentStore` trait

Plus a prior quick fix: human-readable byte formatting in `BufferStats` Display (user chose `4.0KB` over `4096B`).

---

## a) FULLY DONE (verified this session)

### Human-readable BufferStats Display (`src/lib.rs`)
- Added `fn format_bytes_human(bytes: u64) -> String` — binary units (`B`, `KB`, `MB`, `GB`, `TB`, `PB`), one decimal place above 1024.
- `BufferStats` Display now outputs `disk=4.0KB/1.0MB` instead of `disk=4096B/1048576B`.
- Lint-clean: `indexing_slicing` avoided via `.get(idx).copied().unwrap_or("B")`, `as_conversions` allowed locally on the `u64 → f64` cast.
- Test updated: `disk=4.0KB/1.0MB` assertion.
- CHANGELOG entry updated to mention human-readable binary units.
- **Committed** in `03dd0e3`.

### `PartialEq` + `Eq` for `SegmentConfig` (`src/lib.rs`)
- Manual `impl PartialEq for SegmentConfig` — all scalar fields compared by value; cipher compared by `Arc::ptr_eq` (pointer identity).
- Rationale: `SegmentCipher` trait does not require `PartialEq` (key comparison is a security concern). Pointer identity is the most honest comparison: two configs sharing the same cipher `Arc` are equal; distinct-but-equivalent cipher instances are not.
- `Eq` derived as empty impl (consistent with `PartialEq` being reflexive for all fields).
- 6 tests: matching defaults, different fields, different flush_policy, cipher None vs Some, cipher same Arc, cipher different Arcs.
- **Uncommitted** (working tree).

### `FlushPolicy::validate()` (`src/lib.rs`)
- Public method on `FlushPolicy` that centralizes the `debug_assert!` constraints for `BatchOrIntervalMin` (`min_batch <= batch_size`, `interval <= max_interval`).
- Called from 3 sites:
  - `SegmentConfigBuilder::flush_at_batch_or_interval_min` — replaced inline `debug_assert!`s
  - `SegmentConfigBuilder::build()` — validates whatever policy was set
  - `open_internal()` — validates the final policy at buffer construction
- In release builds, `validate()` is a no-op (the `debug_assert!`s compile away).
- 4 tests: all-valid-variants pass, boundary values pass, `min_batch > batch_size` panics (debug only), `interval > max_interval` panics (debug only).
- **Pre-existing test fixed:** `batch_or_interval_min_flushes_at_batch_size` had `min_batch: 100 > batch_size: 4` which now panics under `validate()` at `open()`. Changed to `min_batch: 3`.
- **Uncommitted** (working tree).

### Sealed `SegmentStore` trait (`src/store.rs`)
- Standard sealed-trait pattern: `mod private { pub trait Sealed {} }` + `pub trait SegmentStore: private::Sealed + Send + Sync`.
- `impl private::Sealed for RealStore {}` added.
- Seal marker re-exported as `pub use private::Sealed as SegmentStoreSealed` under `#[cfg(any(test, feature = "loom"))]`.
- Three implementors updated:
  - `RealStore` (`src/store.rs`) — `impl private::Sealed`
  - `HookedStore` (`src/tests.rs`) — `impl store::SegmentStoreSealed`
  - `MockStore` (`tests/loom.rs`) — `impl SegmentStoreSealed`
- `lib.rs` loom re-export updated: `pub use store::{RealStore, SegmentStore, SegmentStoreSealed}`.
- AGENTS.md updated: three-layer separation paragraph now documents the seal + re-export mechanism.
- Trait doc comment gained `# Sealed` section.
- **Uncommitted** (working tree).

### Verification (all run this session)
- `cargo fmt --all -- --check` — clean
- `cargo clippy --all-targets -- -D warnings` — clean
- `cargo clippy --all-targets --features encryption -- -D warnings` — clean
- `cargo clippy --features fuzz --all-targets -- -D warnings` — clean
- `cargo test` (default): **145 unit + 1 integration + 34 doctest = 180 pass**
- `cargo test --features encryption`: **167 unit + 1 integration + 39 doctest = 207 pass**
- `cargo doc --no-deps --features encryption` — clean
- `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release` — **12 pass (219s)**
- `scripts/check-html-root-url.sh` — OK (0.5.5 matches)
- `cargo audit` / `cargo deny` — not installed locally (CI handles these)
- **CI:** last 4 runs on `master` are `success` — BUT the current uncommitted changes have NOT been pushed yet, so CI has not seen them.

---

## b) PARTIALLY DONE

Nothing is partially done. All three code quality items are fully implemented and tested.

---

## c) NOT STARTED (from TODO_LIST.md, 15 items remain)

### Durability (release-scoped)
1. ~~Flip default `DurabilityPolicy` from `Segment` to `Throughput`~~ done — shipped in v0.6.0 — **highest-impact remaining TODO**, changes default behavior, needs release scope.

### Testing & concurrency coverage
2. ~~Loom coverage for `for_each_from` snapshot-then-release-lock pattern~~ done — shipped in v0.5.6
3. ~~Loom test for `iter_from`~~ done — shipped in v0.5.6
4. ~~Property test for `publish_disk_stats` correctness~~ done — shipped in v0.5.6
5. ~~Property test for `delete_acked` idempotency under concurrent `append`~~ done — shipped in v0.5.6
6. ~~Stress test for `segment_size_stats` under concurrent `flush` + `delete_acked`~~ done — shipped in v0.5.6
7. ~~Fuzz target for `for_each_from`~~ done — shipped in v0.5.6 (not in CI fuzz matrix — tracked in TODO_LIST)

### Benchmarks
8. ~~`bench_segment_size_stats` (O(n_segments) scan cost)~~ done — shipped in v0.5.6 (never run — tracked in TODO_LIST)
9. ~~`bench_cipher` (encryption overhead vs no-cipher baseline)~~ done — shipped in v0.5.6 (never run — tracked in TODO_LIST)

### Documentation
10. ~~Visually verify README rendering (standing item, user action)~~ done — verified by user 2026-08-10

### CI / process
11. ~~Audit CI vs local gate parity~~ done — shipped in v0.5.7
12. ~~Add clippy with full lint stack to MSRV CI job~~ done — shipped in v0.5.7
13. ~~Improve `check-changelog-links.sh` robustness (rate-limit + GITHUB_TOKEN)~~ done — shipped in v0.5.7
14. ~~Add `--list` and `--only=` options to `verify-gate.sh`~~ done — shipped in v0.5.7

---

## d) TOTALLY FUCKED UP

### Nothing is fucked up, but two things to call out:

1. **CHANGELOG test count was stale AGAIN.** The "Internal deduplication" entry said "184 tests" — actual verified count is 145 (default). Fixed to "145 tests (default features)". This is the same class of drift that was called out last session. The root cause: the CHANGELOG entry was written mid-session before the session's own test additions landed, and the count was never re-verified. **The verification discipline rule 2 ("never invent baselines") applies to test counts too.**

2. **Pre-existing test violated the new validation constraint.** `batch_or_interval_min_flushes_at_batch_size` had `min_batch: 100` with `batch_size: 4` — exactly the invalid configuration `validate()` now catches. This wasn't a "fuck up" per se (the test predated `validate()`), but it reveals that the constraint was never enforced anywhere before, so invalid configs could silently reach production callers. The test was fixed to use `min_batch: 3`.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

1. **Commit after each task, not after all three.** The auto-git daemon will likely commit the working tree as a single blob. Three independent features (PartialEq, validate, seal) deserve three commits for bisectability. This is the same lesson from last session ("commit after each phase group").

2. **The CHANGELOG test-count drift is a recurring pattern.** Every session that adds tests also edits the CHANGELOG, and the count is always stale by the time it's verified. Consider either (a) omitting exact test counts from CHANGELOG entries (they're meaningless to downstream readers), or (b) adding a `grep -c '#\[test\]'` assertion to `verify-gate.sh` that cross-checks against the CHANGELOG claim.

3. **The `FlushPolicy::validate()` method is public but only does anything in debug builds.** This is the correct design (release builds should not panic on caller mistakes), but the doc could be more explicit that release-mode callers get zero protection. A future improvement: make `validate()` return a `Result<(), ConfigError>` variant for release-mode enforcement, keeping the debug-panic as a `debug_assert!` inside.

4. **The sealed trait re-export mechanism is clever but non-obvious.** `SegmentStoreSealed` is a type alias for `Sealed` that only exists under `cfg(any(test, feature = "loom"))`. A downstream contributor who tries to implement `SegmentStore` will get a confusing error ("trait `Sealed` is private"). The error message could be improved by adding a compiler diagnostic note via `#[diagnostic::on_unimplemented]` (stable in Rust 2024 edition) — but that requires an edition bump, so it's a future item.

### Code quality observations

5. **The `NopCipher` test helper is duplicated three times** in `segment_config_partial_eq_*` tests. It should be extracted to a shared test helper at the top of the test module, like `prop_item` in `property_tests.rs`.

6. **`format_bytes_human` is a private free function.** If users want to format byte counts in their own logging (e.g., for `segment_size_stats`), they can't reuse it. Consider promoting to `pub(crate)` or making it a method on a new `ByteSize(u64)` newtype — but that's scope creep for now.

7. **`FlushPolicy::validate()` only validates `BatchOrIntervalMin`.** The other variants (`Batch`, `Interval`, `BatchOrInterval`, `Manual`) have no constraints to validate today. But if future variants add constraints, `validate()` is the single place to add them — the architecture is right even if coverage is minimal.

---

## f) Up to 50 things to get done next

> **Harvested (2026-08-10).** Actionable items extracted into `TODO_LIST.md`.
> Remaining items are aspirational brainstorm, not tracked work.

### High impact (ship value or close risk gaps)
1. **Flip `DurabilityPolicy` default to `Throughput`** — highest-impact TODO, release-scoped
2. **Loom coverage for `for_each_from`** — highest-risk correctness gap (new snapshot pattern has no exhaustive proof)
3. **Loom test for `iter_from`** — materialising iterator path has no dedicated loom proof
4. **Property test for `publish_disk_stats`** — verify atomic counters match reality
5. **Property test for `delete_acked` idempotency under concurrent `append`** — complement existing loom proof
6. **Stress test for `segment_size_stats` under concurrent flush + delete** — scan safety under mutation
7. **Fuzz target for `for_each_from`** — arbitrary start + limit + concurrent mutations
8. **Ship v0.5.6 or v0.6.0** — the `[Unreleased]` block is now substantial (Display impls, PartialEq, validate, sealed trait, human-readable bytes, property tests, doc sections)

### Code quality (from this session's observations)
9. **Extract `NopCipher` to a shared test helper** — duplicated 3× in `src/tests.rs`
10. **Consider `ByteSize(u64)` newtype** — promote `format_bytes_human` to reusable public API
11. **Consider `FlushPolicy::validate()` returning `Result`** — release-mode enforcement for callers who want it
12. **Add `#[diagnostic::on_unimplemented]` on `SegmentStore`** — better error message for sealed-trait violations (requires edition 2024)

### Benchmarks
13. **`bench_segment_size_stats`** — quantify O(n_segments) scan at 100/1k/10k segments
14. **`bench_cipher`** — AES-GCM / XChaCha20 overhead vs no-cipher baseline
15. **`bench_format_bytes_human`** — trivial but consistent with the crate's "everything has a bench" posture

### Documentation
16. **Visually verify README rendering** on GitHub + docs.rs + mobile (standing item, user action)
17. **Document the sealed-trait pattern in CONTRIBUTING.md** — so future contributors understand why `SegmentStore` can't be implemented externally
18. **Add `FlushPolicy::validate()` to the crate-level examples** — show the validation pattern in the rustdoc
19. **Update `docs/DOMAIN_LANGUAGE.md`** — add `PartialEq` semantics for `SegmentConfig` (pointer identity for cipher)
20. **Consider a "Configuration validation" section in the rustdoc** — explain `validate()` and when it runs

### CI / process
21. **Audit CI vs local gate parity** — enumerate and diff every check
22. **Add clippy with full lint stack to MSRV CI job** — currently only `cargo check`
23. **Improve `check-changelog-links.sh`** — rate-limit detection + GITHUB_TOKEN support
24. **Add `--list` and `--only=` to `verify-gate.sh`** — faster iteration
25. **Add test-count cross-check to `verify-gate.sh`** — prevent CHANGELOG drift
26. **Run `scripts/verify-gate.sh --all` before next release tag** — full 15-gate pass
27. **Check `gh run list` before ANY "done" claim** — verification discipline rule 10

### Testing infrastructure
28. **Add property test for `format_bytes_human`** — roundtrip: parse output back to approximate byte count
29. **Add test for `format_bytes_human` edge cases** — 0, 1, 1023, 1024, 1025, u64::MAX
30. **Add test for `SegmentConfig::PartialEq` with `FlushPolicy::Manual`** — verify all policy variants compare correctly
31. **Add test for `FlushPolicy::validate()` on non-`BatchOrIntervalMin` variants** — confirm they're no-ops
32. **Consider a test that `validate()` is called during `open()`** — integration test with invalid config that should panic in debug

### Architecture / future
33. **Envelope v2 design** — streaming CBOR early-stop, Blake3 checksum, compression negotiation, cipher auto-detection (long-term, blocked on format change)
34. **Streaming/incremental cipher** — bound memory on large segments (long-term, format change)
35. **Second `SegmentStore` impl** — e.g., in-memory store for testing or S3-backed store (long-term)
36. **Async I/O support** — long-term direction in ROADMAP.md
37. **Nightly benchmark CI workflow** — track perf regressions automatically

### Polish
38. **Add `PartialEq` for `BufferStats`** — natural complement to `SegmentConfig` PartialEq; all fields are `Copy` types so derive works directly
39. **Add `PartialEq` for `RecoveryReport`** — same reasoning
40. **Consider `Hash` for `FlushPolicy`** — enables use in `HashMap` keys for test infrastructure
41. **Consider `Hash` for `DurabilityPolicy`** — same
42. **Add `Display` for `RecoveryReport`** — logging-friendly summary of recovery state
43. **Review all `debug_assert!` sites** — ensure they're all reachable via `validate()` or equivalent centralized checks
44. **Consider making `format_bytes_human` `const`** — currently impossible due to format!, but worth tracking
45. **Add `FromStr` for `DurabilityPolicy`** — parse `"maximal"` / `"segment"` / `"throughput"` from config files
46. **Add `FromStr` for `FlushPolicy`** — parse Display format back (roundtrip)
47. **Review whether `SegmentConfigBuilder` should validate on every setter** — currently only `flush_at_batch_or_interval_min` and `build()` validate
48. **Consider `SegmentConfigBuilder::validate()` method** — let callers check before `build()`
49. **Add a "Migration guide" section to CHANGELOG** — for the eventual DurabilityPolicy default flip
50. **Review the `NopCipher` pattern for a potential test-only export** — `fuzz_hooks` exports test helpers; consider a `test_helpers` module for cipher stubs

---

## g) Questions (cannot figure out myself)

### ~~1. Should the next release be v0.5.6 (additive-only) or v0.6.0 (bundle DurabilityPolicy default flip)?~~ done — resolved: v0.5.6 and v0.6.0 both shipped

The `[Unreleased]` changes are purely additive: Display impls, PartialEq/Eq, validate(), sealed trait, human-readable bytes, property tests, doc sections. These are all patch-level (semver-compatible). But TODO item #1 (DurabilityPolicy default flip) is the highest-impact remaining item and changes default behavior (minor-level). Should we:

- **(a)** Ship v0.5.6 now with the additive changes, then v0.6.0 later with the DurabilityPolicy flip? (Two releases, clean semver.)
- **(b)** Bundle everything into v0.6.0? (One release, but the DurabilityPolicy flip hasn't been written yet.)
- **(c)** Ship v0.5.6 now, defer the DurabilityPolicy flip to v0.6.0 with a deprecation warning in v0.5.6?

### ~~2. Should the uncommitted working tree changes be committed now, or should I continue with more TODO items first?~~ done — resolved: committed and shipped in subsequent releases

The auto-git daemon is active and will commit eventually. But committing now (or letting the daemon do it) means CI hasn't seen these changes yet. Should I:

- **(a)** Commit now and wait for CI to confirm green before doing anything else?
- **(b)** Continue working on more TODO items and let the daemon handle commits?
- **(c)** Commit now, then immediately start the next TODO item without waiting for CI?

### ~~3. Should `FlushPolicy::validate()` return `Result<(), SegmentError>` in release builds too?~~ done — resolved: kept debug-only (matches Rust conventions); future Result variant tracked as aspirational

Currently `validate()` panics in debug, no-ops in release. This matches the pre-existing `debug_assert!` behavior. But a caller who constructs an invalid `BatchOrIntervalMin` in production gets no warning until the policy behaves incorrectly. Should:

- **(a)** Keep as-is (debug-only enforcement — matches Rust conventions for internal invariants)?
- **(b)** Add a `validate_checked() -> Result<(), SegmentError>` variant for release-mode callers who want explicit error handling?
- **(c)** Make `validate()` return `Result` in all builds, with debug-mode still panicking via `.unwrap()` internally?

---

## Session metrics

| Metric | Value |
|--------|-------|
| Files modified | 7 (+318 / -38 lines) |
| New tests added | 10 (4 validate, 6 PartialEq) |
| Total tests (default) | 145 unit + 1 integration + 34 doctest = 180 |
| Total tests (encryption) | 167 unit + 1 integration + 39 doctest = 207 |
| Loom tests | 12 (219s) |
| TODO items completed | 3 (Code quality section fully eliminated) |
| TODO items remaining | 15 |
| CI status (last push) | success (6m10s) — has NOT seen uncommitted changes |
| Gates verified this session | fmt, clippy×3, test×2, doc, loom, html_root_url |
| Gates NOT verified this session | cargo-audit, cargo-deny (not installed locally), lychee, changelog-links, actionlint, nix flake check |

---

## Raw verification output

```
$ cargo test
test result: ok. 145 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test --features encryption
test result: ok. 167 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 219.30s

$ cargo clippy --all-targets -- -D warnings          # clean
$ cargo clippy --all-targets --features encryption   # clean
$ cargo clippy --features fuzz --all-targets         # clean
$ cargo fmt --all -- --check                          # clean
$ cargo doc --no-deps --features encryption           # clean
$ scripts/check-html-root-url.sh                      # OK (0.5.5)

$ gh run list --limit 4
completed  success  Fuzz   master  schedule  6m29s
completed  success  Nix    master  push       3m16s
completed  success  CI     master  push       4m33s
completed  success  CI     master  push       6m10s
```
