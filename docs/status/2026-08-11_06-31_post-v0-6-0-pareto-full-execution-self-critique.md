# Status Report: Post-v0.6.0 Pareto Plan — Full Execution

**Date:** 2026-08-11 06:31 CEST
**Session start:** ~05:15 CEST
**Author:** Crush execution session
**Starting commit:** `5897050` (docs: add post-v0.6.0 Pareto plan)
**Ending commit:** `ccd272a` (docs: rebuild TODO_LIST after Pareto plan execution)
**Working tree:** Clean. On `master`, up to date with `origin/master`.
**CI:** Green on all 4 most recent runs (CI + Nix, across the last 2 pushes).
**Plan:** [`docs/planning/2026-08-11_05-12_post-v0-6-0-pareto-code-quality-ci-hardening.md`](../planning/2026-08-11_05-12_post-v0-6-0-pareto-code-quality-ci-hardening.md)

---

## What this session did

Executed the entire post-v0.6.0 Pareto plan — phases P0 through P6 —
covering release safety, benchmark honesty, quick code wins, CI hardening,
documentation polish, testing expansion, and new benchmark targets. Six
commits produced across 4 logical batches, pushed and CI-verified.

---

## a) FULLY DONE

### P0 — Release Safety (2 tasks)

1. **Cargo.lock version-sync gate** (`scripts/check-cargo-lock-version.sh`).
   New script extracts the version from Cargo.toml and Cargo.lock, asserts
   equality. Registered in `verify-gate.sh` as the `cargo-lock-version` gate
   (gate 18 of 18). This would have caught the v0.5.7 publish failure where
   Cargo.lock wasn't synced after the version bump. Tested: `--only=cargo-lock-version`
   passes. The script is executable, uses the same `set -euo pipefail` +
   `cd "$(dirname "$0")/.."` pattern as `check-html-root-url.sh` and
   `check-msrv.sh`.

2. **Release runbook updated** (AGENTS.md step 3). Added `cargo check` (syncs
   Cargo.lock) and `cargo publish --dry-run --features encryption` (catches
   packaging errors before tagging). Gate count references updated 17→18
   in the runbook and in verification-discipline rule 4.

### P1 — Benchmark Honesty (5 tasks)

3. **`bench_segment_size_stats` executed for the first time.** Produces
   sensible output: 57.5 us (100 segments), 437.8 us (1k), 10.2 ms (10k).
   No panic, no crash. This benchmark shipped in v0.5.5 and had never been
   run until this session.

4. **`bench_cipher` executed for the first time** (with `--features encryption`).
   Produces sensible output: no-cipher 79.9 us, AES-256-GCM 87.2 us,
   XChaCha20-Poly1305 114.5 us. This benchmark shipped in v0.5.0 and had
   never been run until this session.

5. **PERFORMANCE.md baseline snapshot refreshed.** Replaced the stale
   v0.5.6/level-3 snapshot with a fresh v0.6.0/level-1 snapshot. All 11
   original benchmarks re-run with `--sample-size 10` and the median values
   recorded. New concurrent-append table added showing `append_all` 3.16x
   advantage at 8 threads.

6. **Compression-sweep analysis doc written.**
   `docs/perf/2026-08-10_compression-level-sweep.md` accompanies the existing
   TSV data with 5 key findings: level-1 default is correct for cloud-sync,
   drain throughput is compression-level-independent, the "knee" is at level
   5-6 for uniform payloads, levels above 15 are impractical, and peak disk
   usage varies with ratio.

7. **Real-disk vs tmpfs finding documented** in PERFORMANCE.md "What is NOT
   measured here" section. The flush path is CPU-bound at level 1 (tmpfs vs
   SSD within 5%). Only at `Maximal` durability does I/O become measurable.

### P2 — Quick Code Wins (3 tasks)

8. **NopCipher extracted.** The identical struct + SegmentCipher impl was
   duplicated 3x in `src/tests.rs` at the `segment_config_partial_eq_*` test
   group. Extracted to a single shared definition at the top of the test
   module. All 3 tests pass with the shared helper.

9. **`format_bytes_human` edge-case tests.** 5 tests covering 0, 1023, 1024,
   1025, and `u64::MAX`. All pass. The u64::MAX test confirms no panic on
   the largest possible input.

10. **Compression-level default regression guard.** Test asserting
    `SegmentConfig::default().compression_level == 1`. Catches accidental
    drift of the v0.5.7 default change.

### P3 — CI Hardening (3 tasks)

11. **`update-flake-lock.yml` permissions fixed.** Added
    `permissions: contents: write, pull-requests: write`. The weekly workflow
    was failing every Monday with 403 because the default `GITHUB_TOKEN`
    has read-only permissions. actionlint passes on the modified file.

12. **All 7 fuzz targets added to CI.** The fuzz.yml matrix expanded from
    2 targets (`fuzz_corrupted_read`, `fuzz_recovery`) to all 7
    (`+ fuzz_append_all, fuzz_envelope, fuzz_flush_policy, fuzz_for_each_from,
    fuzz_parse_filename`). This ~3.5x's the nightly fuzz CI time (~6 min →
    ~21 min) but gives full coverage.

13. **Full 18-gate verification run completed.** 15/15 non-supply-chain,
    non-loom gates green via `verify-gate.sh --no-supply-chain --no-loom`.
    Supply-chain (cargo-deny + cargo-audit) run separately: both pass.
    Loom gate run separately: 14 tests pass in 219.97s.

### P4 — Documentation Polish (4 tasks)

14. **LIMITATIONS.md status tags.** All 18 limitation headings now carry
    inline status badges: _(Permanent)_, _(Tradeoff)_, or _(Roadmap:
    envelope v2)_. A legend was added to the intro explaining each tag.

15. **FEATURES.md unit-test cell split.** The 2000+ character run-on cell
    was replaced with a concise summary + a "Test coverage detail" section
    below the table listing all categories.

16. **AGENTS.md "Durability model" section verified** — already coherent
    after v0.6.0 flip. No stale references found. No changes needed.

17. **DOMAIN_LANGUAGE.md compression_level verified** — already documents
    default as 1 since v0.5.7. No changes needed.

### P5 — Testing Expansion (3 tasks)

18. **`PartialEq` derive added to `BufferStats`.** All fields are `Copy`,
    so the derive is trivial. Added 2 equality tests (same-values-equal,
    different-values-not-equal). Clippy clean.

19. **`flock_released_on_drop` test.** Opens a buffer, drops it, opens the
    same directory again — verifies the lock was released on Drop. Passes.

20. **`tests/cross_platform_fs.rs` integration test module.** 2 tests
    documenting cross-platform filesystem semantics: concurrent unlink on
    the same path (idempotent delete_acked), and concurrent delete_acked on
    overlapping ranges. Both pass on Linux ext4.

### P6 — New Benchmark Targets (5 tasks)

21. **`bench_iter_from`** — compares `iter_from` (materialising iterator)
    vs `for_each_from` (callback) vs `read_from` (owned Vec) at 1k and 10k
    items. Registered in Cargo.toml, compiles clean, produces output.

22. **`bench_flush`** — isolates the `flush()` encode pipeline (CBOR + zstd +
    write) at 100/1k/10k items. Uses `iter_batched_ref` for per-iteration
    setup. Produces sensible output (flush_100: 32.7 us).

23. **`bench_concurrent_read`** — `read_from` under 1/2/4/8 concurrent
    reader threads. Produces sensible output.

24. **`bench_mixed_read_write`** — producer + consumer threads concurrently,
    modelling the cloud-sync workload. 1/2/4 producers with 1 consumer.
    Compiles and produces output.

25. **`bench_append_realistic`** — `append` with uniform/text/JSON payloads.
    Added `text_item` and `json_item` helpers to `benches/support.rs`.
    Produces output showing payload-entropy impact on throughput.

### Cleanup

26. **TODO_LIST.md rebuilt.** All 10 original items removed (all completed).
    3 new items added from the Pareto plan's deferred section (D5, D6, D9).
    Pareto plan status updated to "EXECUTED".

27. **Test/bench counts updated** across all living docs. FEATURES.md:
    132→141 unit tests, 11→16 benchmarks. AGENTS.md: same. PERFORMANCE.md:
    available-benchmarks table expanded with all 5 new entries.

---

## b) PARTIALLY DONE

### PERFORMANCE.md baseline is incomplete for the 5 new benchmarks

The baseline snapshot (item 5 above) refreshed the **original 11 benchmarks**
under v0.6.0/level-1. The **5 new benchmarks** (P6) have no baseline snapshot
section yet — they were verified to produce output, but no results table was
added to PERFORMANCE.md. The available-benchmarks table lists them, but the
"Baseline snapshot" section does not include their numbers.

### `bench_mixed_read_write` has a design concern

The consumer thread's `read_from(0, n_items)` loop with `delete_acked` is
semantically questionable: `read_from(0, ...)` always starts from seq 0, but
after `delete_acked`, those segments are gone and `head_sequence` has
advanced. The benchmark compiles and runs (the consumer eventually completes
because `read_from` returns empty once everything is consumed and deleted),
but the consumer's logic is fragile and may not accurately model a real
cloud-sync drain loop. This is a benchmark correctness concern, not a
library bug.

---

## c) NOT STARTED

### Deferred items (tracked in ROADMAP.md, explicitly out of scope for this plan)

- Envelope v2 (streaming CBOR, Blake3 checksum, cipher auto-detection)
- Streaming/incremental cipher (RFC 8450)
- Second `SegmentStore` impl (S3-backed)
- Async I/O exploration
- `FlushPolicy::validate()` Result-returning variant
- `ByteSize(u64)` newtype
- `BatchOrIntervalMin` as default FlushPolicy
- Nightly benchmark CI workflow (D5 — now in TODO_LIST)
- jscpd duplication gate (D6 — now in TODO_LIST)
- `Hash` for `FlushPolicy` + `DurabilityPolicy` (D9 — now in TODO_LIST)

These were correctly deferred — they are multi-day efforts or format changes
that require a concrete consumer.

---

## d) TOTALLY FUCKED UP

### 1. No CHANGELOG `[Unreleased]` entry

**This is the biggest miss.** The session added 9 new tests, 5 new benchmarks,
a new verify-gate script, `PartialEq` on `BufferStats`, fixed 2 CI workflows,
and expanded the fuzz matrix. CHANGELOG.md `[Unreleased]` is **empty**. Every
one of these changes is user-visible or contributor-visible. The release
runbook says "move `[Unreleased]` entries under a new version heading" — but
there's nothing to move. The next release will ship with no changelog notes
for this work unless someone remembers to add them retroactively.

**Root cause:** tunnel vision on executing the Pareto plan phases in order.
CHANGELOG was not in the plan (the plan predated the work it generated), so
it was never added as a task.

### 2. PERFORMANCE.md baseline section was not updated after P6 benchmarks were created

The baseline snapshot was refreshed under P1c with all 11 original benchmarks.
Then P6 added 5 more benchmarks. The baseline section was NOT updated to
include the new benchmarks' results. The available-benchmarks TABLE lists all
16, but the baseline SNAPSHOT section only has the original 11. This is an
internal inconsistency.

### 3. The `bench_mixed_read_write` consumer logic is questionable

The consumer thread calls `read_from(0, n_items)` in a loop, but after the
first `delete_acked`, subsequent `read_from(0, ...)` calls start from a seq
that's already been deleted. The benchmark "works" (it eventually finishes),
but the consumer loop may be measuring retry overhead rather than real
mixed-workload throughput. A correct version would track a cursor and call
`read_from(cursor, ...)` like a real drain loop.

### 4. Cross-platform test module has no `#[cfg(...)]` platform guards

`tests/cross_platform_fs.rs` documents platform differences in its module
doc comment, but the tests themselves are not cfg-gated or platform-aware.
They pass identically on all platforms because they test idempotent behavior
(the library handles both "ENOENT on second unlink" and "both succeed"
correctly). This is fine for now, but the module doc comment promises
platform-specific testing that the tests don't actually deliver. The tests
are regression guards for the v0.5.6 APFS issue, not platform-difference
validators.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

1. **CHANGELOG discipline.** Every batch of changes should update `[Unreleased]`
   in the same commit. The Pareto plan should have included "update CHANGELOG"
   as a meta-task after each phase. This is the #1 process failure this
   session.

2. **Baseline snapshot after new benchmarks.** When P6 added 5 new benchmarks,
   the baseline snapshot should have been extended immediately. Instead the
   snapshot section was completed under P1c and then P6 added benchmarks
   without backfilling the snapshot. The fix is to treat "add benchmark" and
   "add benchmark baseline to PERFORMANCE.md" as a single task.

3. **Benchmark correctness review.** The `bench_mixed_read_write` consumer
   loop was written quickly to model a cloud-sync workload. It should have
   been reviewed against the actual `examples/cloud_sync.rs` drain loop
   pattern before committing. Benchmark code that measures the wrong thing
   is worse than no benchmark.

4. **The auto-git daemon committed P6 before manual commit.** The daemon
   committed the benchmark files as `4207080` with a decent message, but
   the working tree had uncommitted doc updates (AGENTS.md, FEATURES.md,
   PERFORMANCE.md bench counts) that were then folded into the next commit.
   The daemon's commit boundary split a logical unit of work across two
   commits. Should have committed immediately after `cargo clippy` passed,
   before running the benchmarks for output verification.

### Code quality observations

5. **`format_bytes_human` is private but tested via the test module.** The
   tests live in `src/tests.rs` which has access to private items via
   `use super::*`. This is correct but means the function is tested as a
   side effect of BufferStats Display testing, not as a standalone unit.
   If the function were ever extracted to a utility module, the tests would
   need to move too.

6. **`cross_platform_fs.rs` uses `SegmentConfig::builder()` correctly** (the
   struct is `#[non_exhaustive]`), but the test's `config()` function
   duplicates the logic in `benches/support.rs::config()`. If the config
   shape changes, both need updating. Consider a shared test-config helper.

7. **The `cargo-lock-version` gate uses `grep -A1` to extract the version
   from Cargo.lock.** This is fragile — if a future Cargo.lock format
   change adds a blank line between `name` and `version`, the script breaks.
   The awk approach (initial attempt) was more robust but had a quoting
   issue. Consider using `cargo metadata --format-version 1 | jq` for a
   truly robust extraction.

---

## f) Up to 50 things we should get done next

### High priority (release-blocking for v0.6.1)

1. **Add CHANGELOG `[Unreleased]` entries** for all changes this session.
2. **Add PERFORMANCE.md baseline snapshot** for the 5 new benchmarks.
3. **Fix `bench_mixed_read_write` consumer logic** — track cursor like a real drain loop.
4. **Verify the 5 new benchmarks produce stable numbers** — `--sample-size 10` is low; run with default sample size once.

### Code quality (quick wins)

5. **Add `Hash` + `Eq` derives** to `FlushPolicy` and `DurabilityPolicy` (D9, now in TODO_LIST).
6. **Add `PartialEq` derive to `SegmentSizeStats`** — same pattern as BufferStats.
7. **Add `Eq` derive to `BufferStats`** — `PartialEq` is there but `Eq` is missing (f32 field prevents it — document why or use `PartialEq` only).
8. **Extract `test_config()` helper** to a shared test-support module to avoid duplication between `src/tests.rs` and `tests/cross_platform_fs.rs`.
9. **Add `NopCipher` to the public docs** under a "testing utilities" section — downstream crates need a no-op cipher for their own tests.
10. **Review `bench_iter_from` for fairness** — the `iter_from` bench counts items in a loop, which includes iterator state-machine overhead that `read_from` doesn't have. Is this an apples-to-oranges comparison?

### CI / process

11. **Add jscpd duplication gate** to CI (D6, now in TODO_LIST).
12. **Add nightly benchmark CI workflow** that commits criterion baselines (D5, now in TODO_LIST).
13. **Monitor the expanded fuzz CI runtime** — 7 targets x 5 min = ~35 min. If this is too slow, implement rotation.
14. **Add a `cargo publish --dry-run` step to CI** on every push to master — catches packaging errors continuously, not just at release time.
15. **Verify the `update-flake-lock.yml` fix works** — wait for the next Monday 04:00 UTC run and confirm it creates a PR instead of 403.
16. **Add `shellcheck` to the Nix devShell** — the new `check-cargo-lock-version.sh` was tested manually but never shellchecked.

### Documentation

17. **Document the `PartialEq` semantics for `BufferStats`** in DOMAIN_LANGUAGE.md — two stats snapshots are equal iff all 8 fields match.
18. **Add "How to add a new benchmark" guide** to AGENTS.md or CONTRIBUTING.md — the pattern (register in Cargo.toml, add to PERFORMANCE.md table, add to FEATURES.md count) is now established but undocumented.
19. **Update `examples/scaling.rs` to support the new payload types** (text, JSON) from `benches/support.rs` — currently the scaling test only does uniform.
20. **Write a "Benchmark interpretation guide"** — explain that `--sample-size 10` numbers are indicative, not statistically rigorous, and how to run full-sample benchmarks for publication.
21. **Add the 5 new benchmarks to FEATURES.md "Performance" section** if one exists, or note them in the testing section.
22. **Cross-reference LIMITATIONS.md status tags with ROADMAP.md** — every _(Roadmap: envelope v2)_ tag should link to the ROADMAP.md item.

### Testing

23. **Add a test for `SegmentConfig::builder().compression_level(22)`** — verify the maximum level is accepted.
24. **Add a test for `SegmentConfig::builder().compression_level(-1)`** — verify negative levels are handled (zstd uses negative levels for "fast" modes).
25. **Add property test: `read_from(start, limit)` never returns more than `limit` items** — currently tested implicitly but not as a standalone property.
26. **Add stress test: 100 threads x 1000 items each** — current max is 8 threads.
27. **Add test: `delete_acked` on empty buffer is a no-op** — may already exist, verify.
28. **Add test: `flush()` on empty buffer is a no-op** — may already exist, verify.
29. **Add test: `iter_from` after `delete_acked` skips deleted segments** — the loom test covers concurrency but not the sequential case.
30. **Add test: `segment_size_stats` after all segments deleted returns zeros** — may already exist, verify.

### Performance investigation

31. **Investigate why `append/batch_1` regressed** — the v0.6.0 baseline shows 83.7 us vs the v0.5.6 baseline's 27.0 us (3x slower). This is likely the `--sample-size 10` noise (criterion's change detector reported +205%), but it warrants investigation with a full-sample run.
32. **Profile the `flush()` encode pipeline** with `cargo flamegraph` — the `bench_flush` numbers (32.7 us for 100 items) suggest CBOR serialization dominates, not zstd.
33. **Benchmark with real disk** (not tmpfs) — the PERFORMANCE.md claim of "CPU-bound" is based on internal testing, not a documented benchmark. Add a `--disk` flag to the scaling example.
34. **Compare zstd level 1 vs level 0 (uncompressed)** — quantify the compression CPU cost in isolation.
35. **Add a `bench_recover_encrypted` variant** — recovery of encrypted segments has different cost characteristics than plaintext recovery.

### Architecture / long-term

36. **Envelope v2 design doc** — draft the metadata block layout (cipher id, checksum type, compression algo, version byte).
37. **Streaming cipher proof-of-concept** — RFC 8450 chunked format, measure memory savings on large segments.
38. **`SegmentStore` trait documentation** — the sealed trait is the extension point for S3-backed storage; document the contract for in-tree implementors.
39. **Evaluate `parking_lot::Mutex` vs `std::sync::Mutex`** — at 8 threads, `append` shows significant contention. Would `RwLock` help for read-heavy workloads?
40. **Consider a `SegmentBuffer::try_append`** — non-panicking variant for the `Result`-returning crowd (though the current API already returns `Result<u64>`).

### Polish

41. **Add `#[doc(cfg(feature = "encryption"))]` to cipher re-exports** — docs.rs will show them as feature-gated.
42. **Review all `unwrap()` calls in bench code** — bench code allows them, but some may hide setup bugs.
43. **Add a CONTRIBUTING.md "Release checklist"** that references the verify-gate and the CHANGELOG update step.
44. **Add `cargo bpf` / `cargo bench --bench ... -- --save-baseline`** to the release runbook — publish baseline data alongside releases.
45. **Consider versioning PERFORMANCE.md snapshots** — each release could archive its snapshot to `docs/perf/` like the compression sweep.
46. **Add a "Changelog hygiene" check to verify-gate** — assert `[Unreleased]` is non-empty if there are uncommitted changes, or empty if working tree is clean (would have caught this session's miss).
47. **Review the 3 new TODO_LIST items for priority** — `Hash` derive is 15 min, jscpd is 30 min, nightly bench CI is 60 min. They could all be done in a single follow-up session.
48. **Add a `make bench-all` or `cargo bench-all` convenience target** — running 16 benchmarks individually is tedious.
49. **Consider a criterion summary report** — criterion produces JSON; a script could aggregate all 16 into a single table.
50. **Review whether `bench_mixed_read_write` should be removed** — if the consumer logic can't be fixed cleanly, a misleading benchmark is worse than no benchmark.

---

## g) Questions

### 1. Should the CHANGELOG `[Unreleased]` entry target a v0.6.1 patch release?

The changes this session are: new tests (no API change), new benchmarks (no
API change), `PartialEq` derive on `BufferStats` (technically a minor SemVer
bump since it adds a trait impl), CI fixes (no code change), and a new
verify-gate script (no published code change). The `PartialEq` derive is the
only user-visible API surface change. Should this be a v0.6.1 patch, or held
for v0.7.0? I cannot determine your release cadence preference.

### 2. Should the 5 new benchmarks be included in the v0.6.1 release or held until they have full-sample baselines?

The benchmarks compile clean, clippy clean, and produce output. But they
were only run with `--sample-size 10` (criterion's minimum), and
`bench_mixed_read_write` has the consumer-logic concern noted in §d.3.
Shipping them gives users more performance surface; holding lets us validate
them more thoroughly first.

### 3. Should I fix the CHANGELOG, PERFORMANCE.md baseline, and `bench_mixed_read_write` issues right now?

All three issues in §d are fixable in this session. I stopped because the
instruction was to report, not continue. If you want them fixed before any
other work, say so. The fixes are: ~20 min for CHANGELOG, ~15 min for
PERFORMANCE.md baseline, ~15 min for bench_mixed_read_write consumer logic.
