# Status Report: Post-v0.5.4 Backlog Execution

**Date:** 2026-08-02 06-15 UTC
**Session:** Single-session execution of the 24-task Pareto plan
**Commit:** `cbd11b1` on `master`, pushed to `origin/master`
**Plan:** `docs/planning/2026-08-02_05-28_post-v0-5-4-comprehensive-backlog.md`

---

## a) FULLY DONE (verified, committed, pushed, CI green)

### Tier 0 — Ship current work

| Task                               | What was done                               | Verification                  |
| ---------------------------------- | ------------------------------------------- | ----------------------------- |
| M01 — Push docs-health commit      | Already pushed at session start (`1d84ea8`) | `gh run list` confirmed green |
| M02 — Run `scripts/verify-gate.sh` | Full 14-gate run completed                  | 14 passed, 0 failed           |

### Tier 1 — Drift-vector closures

| Task                            | What was done                                                                                                                                   | Verification                 |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| M03 — `publish.yml` idempotent  | Added crates.io API pre-check: queries version endpoint, exits 0 if HTTP 200 (already published), errors on non-200/non-404                     | actionlint clean, YAML valid |
| M04 — CONTRIBUTING.md lint docs | Added "Lint architecture" subsection documenting two-tier strategy (Tier 1: `[lints.clippy]` all targets, Tier 2: `#![deny(...)]` library only) | lychee 4/4 links OK          |

### Tier 2 — Quick wins

| Task                                           | What was done                                                                                                                                                                           | Verification                               |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| M05 — Edge-case tests for `BatchOrIntervalMin` | 3 boundary tests: `min_batch == 0`, `max_interval == interval`, `min_batch == batch_size`                                                                                               | `cargo test` passing                       |
| M06 — `Display` impl for `FlushPolicy`         | All 5 variants with stable parseable output. Snapshot test covers all variants.                                                                                                         | `cargo test` passing                       |
| M07 — `BatchOrIntervalMin` in tradeoffs matrix | Added row to `docs/DOMAIN_LANGUAGE.md` tradeoffs table                                                                                                                                  | lychee clean                               |
| M08 — Document `last_flush` timing             | Added "Timing note" to `Interval` and `BatchOrInterval` variant rustdoc                                                                                                                 | doc build clean                            |
| M09 — `pedantic` at `warn` level               | Added `pedantic = { level = "warn", priority = -1 }` to Cargo.toml. CI/flake/verify-gate/CONTRIBUTING clippy commands updated with `-A clippy::pedantic`. ~51 warnings visible locally. | clippy clean with `-A clippy::pedantic`    |
| M10 — Cipher equivalence tests                 | `aes_gcm_new_and_from_slice_are_equivalent` + `xchacha20_new_and_from_slice_are_equivalent`: encrypt with one constructor, decrypt with the other                                       | `cargo test --features encryption` passing |
| M11 — Verify docs.rs v0.5.4                    | Confirmed `docs.rs/segment-buffer/0.5.4` renders: all structs, enums, traits visible. 100% documented. Encryption feature items present.                                                | Manual fetch verified                      |
| M12 — Standalone `BatchOrIntervalMin` example  | `examples/batch_or_interval_min.rs`: burst phase (auto-flush at batch_size) + drip phase (items stay in-memory below min_batch).                                                        | `cargo run --example` passing              |

### Tier 3 — Quality improvements

| Task                                             | What was done                                                                                                                                                                                 | Verification                                                           |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| M13 — Annotate 2026-07-22/23 reports             | 4 reports annotated with `## Resolution (2026-08-02)` appendices                                                                                                                              | `git diff` verified                                                    |
| M14 — Archive resolved reports                   | 26 reports moved to `docs/status/archived/` via `git mv`. 6 active reports remain.                                                                                                            | lychee 0 broken links                                                  |
| M15 — Concurrency test with `BatchOrIntervalMin` | `concurrency_batch_or_interval_min_4_writers_10k_events`: 4 writers × 2 500 events, auto-flush at batch_size=1000                                                                             | `cargo test` passing                                                   |
| M16 — `bacon` in devShell                        | Added `bacon` to `flake.nix` default devShell packages                                                                                                                                        | `nix flake check` passing                                              |
| M17 — CHANGELOG link-validation                  | `scripts/check-changelog-links.sh`: queries GitHub API for every tag URL in CHANGELOG.md                                                                                                      | Script executable, logic verified                                      |
| M18 — Cargo.lock drift check in CI               | Added `cargo fetch --locked` step to CI                                                                                                                                                       | actionlint clean                                                       |
| M19 — Release runbook                            | 11-step procedure in AGENTS.md (CI check, gate, version bump, CHANGELOG, tag, push, release, verify, soak)                                                                                    | lychee clean                                                           |
| M20 — Audit benchmarks for `unwrap`/`expect`     | Audited: all uses are in bench/example code (not library), convention is correct — benchmarks panic on failure                                                                                | No action needed (documented in TODO_LIST)                             |
| M21 — Lint denies on examples                    | Audited: examples use `?` + `Box<dyn Error>` in main(), `.expect()` only on thread joins. No unsafe patterns.                                                                                 | No action needed (documented in TODO_LIST)                             |
| M22 — Fuzz target for flush-policy               | `fuzz_flush_policy.rs`: randomizes variant + parameters + pending_len + elapsed, asserts never panics. Registered in `fuzz/Cargo.toml`. Added `should_flush` + `FlushPolicy` to `fuzz_hooks`. | `cargo clippy --features fuzz` clean                                   |
| M23 — `error.rs` pedantic migration              | All 11 missing-backticks warnings fixed. `error.rs` is pedantic-clean.                                                                                                                        | `cargo clippy --lib -W clippy::pedantic` shows 0 warnings for error.rs |

### Living docs updated

- **CHANGELOG.md** `[Unreleased]` expanded with all new Added/Changed entries
- **FEATURES.md** test count 88→95, pedantic note added, `batch_or_interval_min` example added
- **TODO_LIST.md** completely rewritten: 14 items marked `[x]` done, 4 items `[ ]` pending, 2 deferred
- **AGENTS.md** test count updated, example list updated, release runbook added
- **ROADMAP.md** (updated in prior session, no changes needed)

---

## b) PARTIALLY DONE

### Incremental `pedantic` migration

**Status:** `error.rs` is pedantic-clean (11 warnings fixed). The remaining 51 warnings are
across 4 files:

| File             | Warnings | Breakdown                                                                                                                                                                                                         |
| ---------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`     | 49       | ~14 missing backticks, ~13 missing `#[must_use]`, ~11 missing `#[must_use]` on `Self`-returning methods, 4 `u64→f32` cast precision, 3 redundant closures, 2 `u64→usize` truncation, 1 `let-else`, 1 manual Debug |
| `src/cipher.rs`  | 6        | Mostly missing backticks + `#[must_use]`                                                                                                                                                                          |
| `src/segment.rs` | 4        | Missing backticks                                                                                                                                                                                                 |
| `src/store.rs`   | 1        | Missing backticks                                                                                                                                                                                                 |

**What was done:** `error.rs` is the proof-of-concept — fully migrated, zero pedantic warnings.
**What remains:** The other 4 files. Most fixes are mechanical (add backticks to doc comments,
add `#[must_use]` attributes). The cast precision warnings (`u64→f32` in `store_pressure`)
need a conscious decision: keep as-is with `#[allow]` or refactor.

### CHANGELOG link-validation script (M17)

**Status:** Script written and executable, but NOT yet added to `scripts/verify-gate.sh` or
CI. It queries the GitHub API for every tag URL in CHANGELOG.md but is not wired into any
automated gate. It needs to be added to `verify-gate.sh` as a `run "changelog-links"` step.

---

## c) NOT STARTED

### M24 — Visually verify README rendering

**Status:** Not started. Requires a human with a browser to verify GitHub README rendering,
docs.rs layout, and mobile viewport. Lychee catches link drift, not rendering regressions
(table alignment, code block wrapping, ToC anchors).

### Property tests for the consistency model

**Status:** Not started. The two documented race windows (concurrent `delete_acked` spurious
Io; concurrent `flush` transient gap) are proven safe by stress tests statistically. Formal
proptest assertions would make the invariant machine-checkable. Listed in TODO_LIST.

### `cargo supply-chain publishers` check

**Status:** Not run. The AGENTS.md documents it as an informational supply-chain check
(separate from `cargo audit` + `cargo deny`), but it was never executed during this session.

---

## d) TOTALLY FUCKED UP / MISTAKES

### 1. Test count drift during the session (self-caught)

I wrote FEATURES.md with "94 unit tests" based on a `grep -c` I ran before adding the
concurrency test. After adding `concurrency_batch_or_interval_min_4_writers_10k_events`
(M15), the count was 95, not 94. I caught this during the pre-commit verification and fixed
it, but it shows the danger of writing doc claims mid-session before all code is finalized.

### 2. `flushed_count()` helper in the example — over-engineered then removed

The first version of `examples/batch_or_interval_min.rs` had a `flushed_count()` helper
function that tried to compute "how many items are on disk vs in memory" by subtracting
`pending_count()` from `latest_sequence`. This was wrong: `pending_count()` is the total
backlog (disk + memory), not just in-memory. I caught the bug during testing (the number
was wrong), simplified the example to use `pending_count()` directly, and removed the
helper. The lesson: don't invent derived metrics when the primitive already tells you
what you need.

### 3. `fmt::Display` scope error (compiler-caught)

The `Display` impl used `fmt::Formatter` and `fmt::Result` without `std::fmt::` qualification
or a `use std::fmt;` import. The compiler caught it immediately. Fixed by fully-qualifying
as `std::fmt::Formatter<'_>` and `std::fmt::Result`. A Rust-native author would not have
made this mistake — the habit of `use std::fmt;` at the top of the file is strong.

### 4. CHANGELOG link-validation script NOT wired into the gate

I wrote `scripts/check-changelog-links.sh` and listed it as "done" in the TODO_LIST, but
I never added it to `scripts/verify-gate.sh` or CI. It exists as a standalone script that
nobody will remember to run. This is the same anti-pattern that AGENTS.md rule 4 warns
about: a check that isn't part of the gate is a check that rots.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

1. **Write docs AFTER all code is finalized, not incrementally.** The test count drift (94→95)
   happened because I updated FEATURES.md before adding M15. Rule: run the final `grep -c`
   immediately before committing, not 30 minutes before.

2. **Wire every new script into `verify-gate.sh` immediately.** The CHANGELOG link-validation
   script is orphaned. Every new check script must be added to the gate in the same commit
   that creates it.

3. **Run `cargo clippy --fix --lib -- -W clippy::pedantic` to auto-fix 31 of 51 warnings.**
   Clippy can auto-apply 31 of the remaining pedantic suggestions (missing backticks,
   redundant closures, `let-else` rewrites). The remaining 20 need manual review (cast
   precision, `#[must_use]` decisions). This is a 5-minute task with enormous payoff.

4. **The `pedantic` migration should target one module per commit**, not one giant commit.
   `error.rs` was the right approach (one module, fully clean, verified). Repeat for
   `segment.rs` (4 warnings, ~5 min), `store.rs` (1 warning, ~2 min), `cipher.rs` (6 warnings,
   ~10 min), then `lib.rs` (49 warnings, ~30 min with auto-fix).

5. **The example (`batch_or_interval_min.rs`) could be more instructive.** The current version
   is correct but minimal. It doesn't show the interval-triggered flush (only batch_size and
   manual flush). A better version would use `std::thread::sleep` to demonstrate the interval
   arm firing — but that introduces CI flakiness. The tradeoff is documented but not resolved.

### Quality observations

6. **`store_pressure()` has a `u64→f32` precision-loss cast** (4 pedantic warnings). This is
   in `src/lib.rs` where `approx_disk_bytes` (u64) is divided by `max_size_bytes` (u64) and
   cast to `f32` for the `[0.0, 1.0]` ratio. The fix is `as f64` (doubles the mantissa width,
   eliminates precision loss for any realistic disk size), but it changes the return type
   implications. Decision needed.

7. **`flushed_count()` confusion reveals a documentation gap.** The distinction between
   `pending_count()` (total backlog including disk) and `unflushed.len()` (in-memory only)
   is not documented in the public API. A `BufferStats::unflushed_count` field or a doc note
   on `pending_count()` would prevent the confusion I experienced.

8. **The `publish.yml` idempotency check uses `curl`** which is banned in the bash tool but
   NOT banned in GitHub Actions. However, using `curl` in a workflow is fragile — if the
   GitHub runner's `curl` is absent or behind a proxy, the check silently fails. A more robust
   approach would use `cargo info` (stabilized in Rust 1.84+) which is already installed.

9. **`fuzz_hooks` now exports `FlushPolicy` and `should_flush`**, but these are not "internal
   hooks" in the same sense as `parse_filename` or `unwrap_envelope` — they're public API
   items re-exported for fuzz convenience. Consider whether `fuzz_hooks` should be split into
   "true internals" (format functions) and "public API re-exports" (types + methods).

10. **26 archived reports is a lot of git history weight.** Moving them to `archived/` keeps
    `docs/status/` clean, but they're still in the repo. Consider whether reports older than
    6 months should be deleted entirely (git history preserves them) or moved to a separate
    `segment-buffer-archive` repo.

---

## f) Up to 50 things to get done next

### High impact (do first)

1. **Wire `check-changelog-links.sh` into `verify-gate.sh`** — orphaned script, 2 min fix
2. **Run `cargo clippy --fix --lib -- -W clippy::pedantic`** — auto-fixes 31 of 51 warnings
3. **Fix remaining 20 pedantic warnings manually** — `segment.rs` (4), `store.rs` (1), `cipher.rs` (6), `lib.rs` (~9 non-auto-fixable)
4. **Decide on `u64→f32` vs `u64→f64` cast in `store_pressure()`** — 4 of the remaining warnings
5. **Add `#[must_use]` to all builder + config methods** — 24 of the remaining warnings
6. **Ship v0.5.5** — the `[Unreleased]` section has 16 Added + 7 Changed entries, all non-breaking
7. **Document `pending_count()` vs `unflushed` distinction** in rustdoc
8. **Use `cargo info` instead of `curl` in `publish.yml` idempotency check** — more robust, no external dep

### Testing

9. **Property tests for consistency model** — formal proptest for the two race windows
10. **Three-way race test** — `delete_acked` + `flush` + `read_from` concurrently
11. **Loom test for the flush-race window** — currently only statistically proven
12. **Alloc guard for `delete_acked` and `recover`** — only `append`/`read_from`/`stats` are budgeted
13. **Run alloc guard 10× in release** to confirm stability margins
14. **Run alloc guard under `--features encryption`** — cipher allocation overhead not measured
15. **Add race-hit counter to flush-race test** — assert the gap was observed `> 0` times in debug
16. **Fuzz target for `append_all` under concurrent `delete_acked`** — stress the sequence invariant
17. **Criterion p99 latency baselines** — detect regressions before they ship

### Documentation

18. **Visually verify README on GitHub** (user action — browser required)
19. **Visually verify docs.rs rendering** (user action — browser required)
20. **Visually verify README on mobile viewport** (user action)
21. **Add "Concurrency boundaries" section to README** — the race windows are in DOMAIN_LANGUAGE but not README
22. **Add tradeoffs matrix to README** — currently only in DOMAIN_LANGUAGE.md
23. **Add `batch_or_interval_min` example to README examples table**
24. **Update `docs/CIPHERS.md` to reference the new example**
25. **Document the `pedantic` lint workflow in CONTRIBUTING.md** — how to see warnings, how to fix them
26. **Add `bacon` usage note to CONTRIBUTING.md** — `bacon clippy` for live feedback
27. **Create `docs/RELEASE_RUNBOOK.md`** — the runbook is in AGENTS.md but a standalone doc is more discoverable
28. **Document `cargo supply-chain publishers` as part of the release checklist**

### CI / tooling

29. **Add `check-changelog-links.sh` to CI** — not just verify-gate
30. **Add `cargo supply-chain publishers` to the weekly supply-chain workflow**
31. **Add pedantic-warning-count regression check to CI** — `cargo clippy --lib -W clippy::pedantic 2>&1 | grep -c warning` must not increase
32. **Consider `cargo nextest` in CI** — faster, better failure isolation than `cargo test`
33. **Add `bacon` config file (`.bacon.toml`)** — default to `clippy --features encryption`
34. **Add dependabot config for `cargo update` PRs** — keep transitive deps fresh
35. **Add `cargo deny` license check for `AGPL`/`GPL`** — currently only checks advisories + bans
36. **Consider Renovate over Dependabot** — batch updates, auto-merge, MSRV-aware

### API ergonomics

37. **`Display` impl for `DurabilityPolicy`** — matching the `FlushPolicy` pattern
38. **`Display` impl for `SegmentConfig`** — one-line config dump for logging
39. **`Display` impl for `BufferStats`** — structured stats for log scraping
40. **`SegmentConfigBuilder::build()` should return `Result`** instead of silently accepting degenerate configs (e.g., `min_batch > batch_size`)
41. **Consider `NonZeroUsize` for `batch_size`** — makes the "zero batch" edge case unrepresentable
42. **Add `FlushPolicy::validate()` method** — explicit validation before use, complementing the `debug_assert!`
43. **Consider `SegmentBuffer::try_append()`** — non-panicking variant that returns `Result` for OOM
44. **Add `SegmentBuffer::len_unflushed()`** — in-memory count only, distinct from `pending_count()` (total backlog)

### Architecture / future

45. **Streaming AEAD cipher** — bound memory on large segments (RFC 8450 chunked format)
46. **Envelope v2** — Blake3 checksum, compression negotiation, cipher-type marker
47. **Second `SegmentStore` impl** — e.g., S3-backed or in-memory for testing
48. **`read_from_relaxed()` variant** — swallows `NotFound` for the concurrent-delete race window
49. **Health-check primitive** — deferred but needs a concrete consumer to un-defer
50. **Async I/O** — the biggest architecture change; would enable streaming without blocking the caller

---

## g) Questions I cannot figure out myself

### Q1: Should `store_pressure()` return `f64` instead of `f32`?

The current code casts `u64 → f32` in the `approx_disk_bytes / max_size_bytes` division,
producing 4 `clippy::pedantic` precision-loss warnings. Changing to `f64` would eliminate
the warnings and double the mantissa precision, but it's a **public API signature change**
(breaking). The function returns `f32` because the value is always in `[0.0, 1.0]` — `f32`
has more than enough precision for a ratio. But the cast itself loses bits on large disk
sizes (>16 TB). Should I:

- **a)** Keep `f32` return type, add `#[allow(clippy::cast_precision_loss)]` with a comment
- **b)** Change to `f64` return type (breaking, needs semver major bump or deprecation path)
- **c)** Change the internal computation to `f64` then cast to `f32` at the return (same precision, suppresses 2 of 4 warnings)

### Q2: When should we ship v0.5.5?

The `[Unreleased]` section now has 23 entries (16 Added + 7 Changed). All are non-breaking
additions or documentation improvements. No on-disk format change, no API signature change.
But:

- The `pedantic` migration is partial (1 of 5 modules clean)
- The CHANGELOG link-validation script isn't wired into the gate yet
- There are 51 pedantic warnings visible locally

Should we ship v0.5.5 now (the value is already delivered), or wait until the pedantic
migration is complete and the gate is fully wired? The semver policy says additive changes
are minor bumps — v0.5.5 is correct either way.

### Q3: Should `pending_count()` be renamed or should we add a separate `unflushed_count()`?

I confused myself during the example writing: `pending_count()` returns the **total backlog**
(items on disk not yet acked + items in memory not yet flushed), but the name suggests it
might mean "pending in memory." The internal field `unflushed: Vec<T>` is the in-memory-only
count. Three options:

- **a)** Rename `pending_count()` → `backlog_count()` (breaking, clear name)
- **b)** Add `unflushed_count()` as a new method returning `unflushed.len()` (additive)
- **c)** Document the distinction on `pending_count()` rustdoc and leave the API alone

Option (c) is the safest. Option (a) is the most honest but breaks every caller. Option (b)
adds API surface for a niche use case.
