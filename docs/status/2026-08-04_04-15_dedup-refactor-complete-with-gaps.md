# Status: Dedup Refactor — Complete but with Honest Gaps

**Date:** 2026-08-04 04:15 CEST
**Session scope:** Code deduplication across `src/lib.rs` and `src/property_tests.rs`
**Branch:** `master`, fully pushed, CI green

---

## a) FULLY DONE

### Library code dedup (`src/lib.rs`)

1. **`BufferInner::pending_count()`** — `const fn`, extracts `unflushed.len() as u64` used by `pub fn pending_count()` and `pub fn stats()`
2. **`BufferInner::latest_sequence()`** — `const fn`, extracts `next_seq.checked_sub(1)` used by `pub fn latest_sequence()` and `pub fn stats()`
3. **`BufferInner::pending_start()`** — extracts `next_seq.saturating_sub(unflushed.len())` used by `read_from`, `for_each_from`, and `delete_acked` (all 3 call sites now use the helper)
4. **`compute_store_pressure(bytes, max)`** — associated fn, extracts `bytes / max clamped to [0,1]` used by `store_pressure()` and `stats()`
5. **`publish_disk_stats(bytes, count)`** — method, extracts the two-atomic-store pattern used by `sync_disk_bytes` and `recover`

### Test code dedup (`src/property_tests.rs`)

6. **`prop_item(id)`** — replaces 20+ inline `PropItem { id: x, payload: format!("payload-{x}") }` constructions
7. **`prop_config(max_size_bytes)`** — replaces 8 inline `SegmentConfig { Manual, ..default() }` blocks
8. **`prop_buffer(dir)`** — replaces 8 inline config+open sequences
9. **`concurrent_test_config()`** — replaces 6 identical inline config blocks for concurrent stress tests
10. **`count_segments(dir)`** — replaces 2 inline `read_dir().map_or(0, ...).count()` blocks

### Verification

- 184/184 tests pass (144 unit + 1 integration + 39 doc)
- 12/12 loom tests pass (219s)
- Clippy clean (`pedantic` + `nursery` + all restriction lints) both with and without `--features encryption`
- `cargo fmt --all -- --check` clean
- `cargo doc --no-deps --features encryption` builds
- CI: SUCCESS on all 7 pushed commits
- Nix flake check: SUCCESS

---

## b) PARTIALLY DONE

### `property_tests.rs` — some inline patterns intentionally left

The following `PropItem { ... }` constructions were **intentionally not converted** because their payload format differs from the standard `"payload-{id}"` pattern:

- Line 228: `PropItem { id: 0, payload: "seed".into() }` — corruption test seed
- Lines 297, 323, 348, 376: `PropItem { id: u64::from(i), payload: format!("p-{i}") }` — short `"p-{i}"` payload used in 4 tests (could use `prop_item` if we don't care about the payload string matching, but the original author chose `"p-"` deliberately for brevity)
- Line 411: `PropItem { id: ..., payload: format!("batch-{batch_idx}-item-{i}") }` — batch-specific payload in `append_all` test

**Assessment:** These are borderline. The `"p-{i}"` ones (4 sites) could arguably be converted to `prop_item(i)` since the tests don't assert on the payload string — only on ids and counts. The batch-specific one is genuinely different.

### `property_tests.rs` — remaining `std::fs::read_dir` inline blocks

Three `std::fs::read_dir(tmp.path())` calls remain that use **different patterns** than simple counting:

- Line ~470: collects `actual_files: Vec<_>` for byte-size comparison
- Line ~838: counts segments with `starts_with("seg_")` instead of `ends_with(".zst")`
- Line ~1128: collects file sizes into a `Vec<u64>` for percentile comparison

**Assessment:** These are genuinely different operations — not duplication.

### AGENTS.md documentation

The AGENTS.md describes the test modules and their helper structures. The new helpers in `property_tests.rs` (`prop_item`, `prop_config`, `prop_buffer`, `concurrent_test_config`, `count_segments`) are not yet documented there. The AGENTS.md "Project layout" section mentions `property_tests.rs` generically.

---

## c) NOT STARTED

### Items identified in the prior session's analysis but not addressed this session

1. **`tests.rs` helper sharing** — `property_tests.rs` now has its own `count_segments` (returns 0 on error), while `tests.rs` has `count_disk_segments` (panics on `read_dir` error). These could theoretically be unified, but they live in separate `mod` blocks and serve different error-tolerance postures.

2. **`read_from` vs `for_each_from` Phase 1 scan duplication** — identified as "acceptable duplication" (different iteration semantics: owned push vs borrowed callback). Not addressed; documented as intentional.

3. **AES-GCM vs XChaCha20 cipher impl similarity** — identified as "acceptable duplication" (different AEAD types make shared helpers more complex than the duplication). Not addressed; documented as intentional.

4. **`prop_buffer` vs `test_buffer` naming inconsistency** — `tests.rs` calls it `test_buffer`, `property_tests.rs` calls it `prop_buffer`. Both do the same thing. Could be unified if helpers were shared, but they're in separate modules.

---

## d) TOTALLY FUCKED UP

**Nothing.** No regressions, no broken tests, no data loss, no force pushes, no lint failures. All 7 commits are clean, pushed, and CI-verified.

The closest thing to a mistake: the auto-git daemon fragmented the work into **7 commits** where 3-4 would have been cleaner. The commit history shows:

- `85f7f65` — initial helper extraction (prior session, unpushed at start)
- `3ea89f8` — const fn fix for clippy (prior session)
- `19dc5ba` — `pending_start` in `delete_acked` + `publish_disk_stats` (this session)
- `9bf7fc3` — property test helpers (this session)
- `c85c5cf` — more property test helper replacements (this session)
- `4a6a8d1` — formatting fix for `count_segments` (this session)

The last three could have been a single commit. The auto-git daemon doesn't squash.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

1. **Run `cargo fmt` before committing, not after.** The `count_segments` function had a formatting issue that `cargo fmt` fixed, but it was committed mid-refactor. Should format-first, commit-second.

2. **Batch replacements more aggressively.** The `replace_all: true` flag on `multiedit` was discovered late. Earlier use would have reduced the 3 property-test commits to 1.

3. **The AGENTS.md "Project layout" section should be updated** to mention the new property test helpers, matching how `tests.rs` helpers are implicitly described.

4. **Cross-module helper sharing** between `tests.rs` and `property_tests.rs` is currently impossible because they're separate `#[cfg(test)] mod` blocks. If test helpers were in a shared `mod test_utils`, both could use them. But this is a structural change with tradeoffs (import noise, less locality).

### Code quality improvements

5. **The 4 remaining `"p-{i}"` PropItem constructions** in `property_tests.rs` could be converted to `prop_item(i)` since no test asserts on the payload string content. This would reduce another 4 inline constructions.

6. **`prop_config` uses `compression_level: 3`** while `concurrent_test_config` uses `compression_level: 1`. This is correct (concurrent tests want speed), but the difference is undocumented in the helper. A doc comment explaining why they differ would help.

7. **`count_segments` in property tests vs `count_disk_segments` in unit tests** — same logic, different error behavior. The property test version returns 0 on error (`map_or(0, ...)`); the unit test version panics (`expect("read_dir")`). Both are correct for their context, but the duplication is a minor smell.

---

## f) Up to 50 Things We Should Get Done Next

### High priority (correctness/durability)

1. **Flip the default `DurabilityPolicy` from `Segment` to `Throughput`** with a deprecation note. The AGENTS.md says this was planned "for one release after the enum lands." v0.5.0 shipped the enum, v0.5.5 is current. Time to flip.
2. **Envelope v2 design** — the cipher type is not recorded in the envelope today; `decode_segment` can only know the cipher by which buffer was opened with. A v2 envelope with cipher-type metadata would close this gap. See `docs/planning/` for the design doc.
3. **Streaming/incremental cipher** — currently the whole segment is buffered (CBOR → zstd → encrypt as blob). A streaming AEAD (RFC 8450 chunked) would bound memory on large segments. Long-term, likely v0.6+.

### Documentation

4. **Update AGENTS.md** "Project layout" section to mention the new `property_tests.rs` helpers (`prop_item`, `prop_config`, `prop_buffer`, `concurrent_test_config`, `count_segments`).
5. **Update AGENTS.md** "Code conventions" section to document the `BufferInner` helper pattern (`pending_count`, `latest_sequence`, `pending_start` as `const fn` where possible).
6. **Add `CHANGELOG.md` entry** for the dedup refactor (7 commits, no changelog entry yet).
7. **Update the prior status report** `docs/status/2026-08-04_03-45_dedup-analysis-and-partial-refactor.md` to mark the remaining items as DONE with commit references.
8. **Clean up the stale status report** `docs/status/2026-08-04_03-45_v0-5-5-released-with-cleanup-gaps.md` — it references "cleanup gaps" that are now closed.
9. **Document the `publish_disk_stats` helper** in the AGENTS.md section on atomic counters.

### Test improvements

10. **Convert the 4 remaining `"p-{i}"` PropItem constructions** to `prop_item(i)` — no assertion depends on the payload string.
11. **Add property tests for `append_all`** edge cases (empty batches interleaved with non-empty, very large batches).
12. **Add property tests for `for_each_from`** covering the snapshot-then-release-lock pattern.
13. **Add property tests for `segment_size_stats`** percentile computation with edge cases (1 element, all-same sizes, very skewed distributions).
14. **Add a property test for `publish_disk_stats`** correctness — verify the atomic counters match reality after `sync_disk_bytes` and `recover`.
15. **Consider sharing `count_segments` / `count_disk_segments`** via a shared `mod test_utils`.
16. **Add a stress test** for `for_each_from` under concurrent `delete_acked` — the re-entrancy removal changed the lock semantics.
17. **Add loom coverage for `for_each_from`** — the snapshot-then-release-lock pattern is new and only covered statistically today.

### Code quality

18. **Extract `SegmentConfigBuilder` methods for common configs** — `prop_config` and `concurrent_test_config` in tests suggest production code might also benefit from config presets.
19. **Consider `#[derive(PartialEq)] for SegmentConfig`** — would make test assertions on config easier.
20. **Add `#[must_use]` to `compute_store_pressure` and `publish_disk_stats`** — they're side-effect-free and side-effectful respectively, both should signal their return/unit.
21. **Review `read_from` Phase 1 / Phase 2 scan duplication** with `for_each_from` — if the snapshot pattern from `for_each_from` could be generalized, `read_from` might benefit.
22. **Consider making `BufferInner` methods `#[inline]`** — `pending_count`, `pending_start`, `latest_sequence` are single-expression helpers that the compiler likely already inlines, but explicit `#[inline]` documents intent.

### CI / Release

23. **Prepare v0.5.6 release** — the dedup refactor is a non-breaking improvement, suitable for a patch release.
24. **Add `cargo-audit` and `cargo-deny` to the local pre-commit gate** — currently CI runs them but `scripts/verify-gate.sh` may not (verify).
25. **Add a CI job for `cargo supply-chain publishers`** — the AGENTS.md documents this as informational; automating it weekly would catch supply-chain drift.
26. **Review the 7-commit history** — consider whether a future `git rebase -i` to squash would be worth it (probably not — history is already pushed).

### Loom / concurrency

27. **Add loom tests for `read_from`** Phase 1 scan + Phase 2 lock gap — currently only covered statistically by stress tests.
28. **Add loom tests for `for_each_from`** snapshot-and-release pattern — new code, only statistically covered.
29. **Add loom tests for `segment_count` self-healing under `sync_disk_bytes`** — the `publish_disk_stats` helper is now shared by two methods; loom coverage on the atomic-store sequence would be valuable.
30. **Consider a loom test for `publish_disk_stats` under concurrent `flush` + `delete_acked`** — the two atomic stores are not a single transaction.

### Examples

31. **Review `examples/background_flush.rs`** for correctness after the `for_each_from` re-entrancy removal.
32. **Add an example demonstrating `DurabilityPolicy` tradeoffs** — `Maximal` vs `Segment` vs `Throughput` with measurable throughput numbers.
33. **Add an example for `segment_size_stats`** batch-size tuning with a real dataset.
34. **Update `examples/backpressure.rs`** to use `compute_store_pressure` pattern if applicable.

### Fuzz / property

35. **Add a fuzz target for `append_all`** — large batches, empty batches, interleaved with flushes.
36. **Add a fuzz target for `for_each_from`** — arbitrary start + limit with concurrent mutations.
37. **Add a property test for `delete_acked` idempotency** under concurrent `append` — the loom tests prove the interleaving, but a property test would cover larger schedules.
38. **Increase proptest case counts** for the concurrent property tests — currently `0u16..200` items, could go higher with `--release`.

### Architecture

39. **Review whether `SegmentStore` trait should be sealed** — it's only reachable under the `loom` feature; sealing would prevent accidental external implementation.
40. **Consider extracting `SegmentConfig` presets** (e.g., `SegmentConfig::throughput_default()`, `SegmentConfig::maximal_default()`) since both tests and production code construct similar configs repeatedly.
41. **Review the `Mutex<Compressor>` pooling** — the compressor pool size is hardcoded; making it configurable would help high-concurrency scenarios.
42. **Consider a `BufferInner::is_empty()` helper** — `unflushed.is_empty() && next_seq == head_seq` is used implicitly in several places.

### Cleanup

43. **Remove the stale `docs/status/2026-08-04_03-45_v0-5-5-released-with-cleanup-gaps.md`** or annotate it — its "cleanup gaps" are now closed by this session.
44. **Annotate `docs/status/2026-08-04_03-45_dedup-analysis-and-partial-refactor.md`** with completion markers — the analysis it contains is now implemented.
45. **Review `TODO_LIST.md`** for items closed by this session and update their status.
46. **Review `FEATURES.md`** — if the dedup refactor changes any user-facing behavior (it doesn't, but verify).
47. **Run the `docs-health` skill** to check for drift across living docs.
48. **Run `scripts/check-msrv.sh`** to verify MSRV consistency after the changes.
49. **Run `scripts/verify-gate.sh`** end-to-end — the full 14-gate local verification script.
50. **Consider archiving old status reports** under `docs/status/archived/` — the directory is accumulating.

---

## g) Questions for the User

1. **Should the 4 remaining `"p-{i}"` PropItem constructions be converted to `prop_item(i)`?** The tests don't assert on the payload string, so the conversion is safe — but it homogenizes the test data, making it harder to distinguish items by payload in debug output. I lean toward converting them, but you may have a preference for keeping the `"p-"` prefix for readability in test failures.

2. **Do you want a v0.5.6 patch release for this dedup work?** It's non-breaking, purely internal. But it's also invisible to consumers (no API change, no behavior change). I'd lean toward waiting and bundling with the next feature release unless you want it out now.

3. **Should `count_segments` (property tests, returns 0 on error) and `count_disk_segments` (unit tests, panics on error) be unified into a shared `mod test_utils`?** This would eliminate the last cross-module duplication but adds a structural change (new module, import management) that may not be worth the churn for two 5-line functions.
