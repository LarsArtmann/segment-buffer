# Status Report: Book Insights Execution — Brutal Self-Review

**Date:** 2026-07-23 17:08
**Session:** Executed the revised book-insights action plan (consistency model, tradeoffs, schema evolution, allocation guard, concurrent-race test)
**Working tree:** UNCOMMITTED — 8 modified files, 2 new files, +634/-151 lines

---

## Executive Summary

The session produced real value: two new tests, three major DOMAIN_LANGUAGE.md sections, crate-level rustdoc, a perf doc, and design deferrals. The verification gate is green (10/10), loom passes (9/9), all 137 tests pass. **But three things are fucked up, and I didn't catch them until this self-review.** Nothing was committed.

---

## a) FULLY DONE

| #   | Item                                                                                                  | Evidence                                                                                                                                                           |
| --- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | **Concurrent read+delete stress test** (`src/tests.rs` +105 lines)                                    | `concurrent_read_and_delete_never_corrupts` — 5× stable in release mode. Proves read_from under concurrent delete_acked never returns corrupt data.                |
| 2   | **Allocation-count guard** (`tests/alloc_guard.rs`, 171 lines new)                                    | Measures allocs on 4 hot paths (warm append: 0, read_from in-mem: 1, stats: 0, append+flush: 27). Budgets: 1/3/1/32. Passes both feature sets.                     |
| 3   | **Consistency Model section** (`docs/DOMAIN_LANGUAGE.md` ~80 lines)                                   | Three subsections: canonical guarantees (read-your-writes, monotonic, consistent-prefix), concurrent operation (two documented race windows), NOT guaranteed list. |
| 4   | **Tradeoffs section** (`docs/DOMAIN_LANGUAGE.md` ~55 lines)                                           | Four tradeable knobs as a matrix, four non-tradeable invariants, two worked examples.                                                                              |
| 5   | **Schema Evolution section** (`docs/DOMAIN_LANGUAGE.md` ~55 lines)                                    | Envelope (crate-managed) vs CBOR payload (caller-managed) distinction. Compatible-change and breaking-change strategies.                                           |
| 6   | **Crate-level rustdoc** (`src/lib.rs` +27 lines)                                                      | Delivery Guarantees + Schema Evolution sections. `cargo doc` renders clean.                                                                                        |
| 7   | **Percentile latency perf doc** (`docs/perf/2026-07-23_percentile-latency-baseline.md`, 83 lines new) | Documents criterion's p99/p99.9 location, extraction command, allocation-guard rationale, pre-release check process.                                               |
| 8   | **Health-check design note** (`TODO_LIST.md` +14 lines)                                               | Three candidate designs, each with Verschlimmbessern risk, deferral verdict.                                                                                       |
| 9   | **ROADMAP cross-reference** (`ROADMAP.md` +15 lines)                                                  | "Reference analyses" section linking mapping + action plan.                                                                                                        |
| 10  | **Action plan execution log** (action plan doc +38/-11 lines)                                         | Acceptance criteria checked off, execution log with deviations from plan.                                                                                          |

---

## b) PARTIALLY DONE

| #   | Item                                   | What's done                                                 | What's missing                                                                                                                                                      |
| --- | -------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Verification gate**                  | All 10 gates green, loom 9/9, 137 tests pass                | Changes are uncommitted. Nothing is pushed. The "done" claim has no commit hash.                                                                                    |
| 2   | **Code examples in Tradeoffs section** | Examples exist, marked `rust,ignore`                        | **They use struct literal syntax that won't compile for external consumers** (see §d). The `rust,ignore` tag masks the error but the examples are still misleading. |
| 3   | **Consistency model documentation**    | Canonical + concurrent sections written, delete-race tested | Flush-race is documented but NOT tested. Only the delete-race has an executable specification.                                                                      |

---

## c) NOT STARTED

| #   | Item                                               | Impact                                                                                                                                                                                                                      |
| --- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Commit**                                         | Zero commit hash exists for this work. Everything is in the working tree.                                                                                                                                                   |
| 2   | **FEATURES.md test count update**                  | Says "Unit tests (82)" — actual count is 83 after adding `concurrent_read_and_delete_never_corrupts`. Documentation drift.                                                                                                  |
| 3   | **CHANGELOG.md entry**                             | No entry for the new tests, new docs sections, or new perf doc.                                                                                                                                                             |
| 4   | **AGENTS.md update**                               | The two-race-window discovery in `read_from` (concurrent delete + concurrent flush) is enduring context that belongs in the "Critical concurrency invariant" section. Not added.                                            |
| 5   | **Flush-race test**                                | Only the delete-race has a test. The flush-race (transient gaps from concurrent flush) is documented but untested.                                                                                                          |
| 6   | **Allocation guard integration into CI narrative** | The test exists but CI doesn't know about it yet. The `ci.yml` runs `cargo test --no-fail-fast --features encryption` which covers it, but FEATURES.md and CONTRIBUTING.md don't mention the allocation guard as a concept. |

---

## d) TOTALLY FUCKED UP

### 1. DOMAIN_LANGUAGE.md Tradeoffs examples use INVALID syntax

**The crime:** I wrote `SegmentConfig { flush_policy: ..., ..SegmentConfig::default() }` in two worked examples. `SegmentConfig` is `#[non_exhaustive]`, so external consumers **cannot use struct literal syntax at all** — only `Default::default()` + field reassignment or the builder.

**The stupidity:** I hit this EXACT error in my own test code (`tests/alloc_guard.rs` got `error[E0639]: cannot create non-exhaustive struct using struct expression`), fixed it there by switching to `let mut config = SegmentConfig::default(); config.flush_policy = ...`, and then wrote the SAME broken pattern in the documentation. I fixed my code but copied the error into the docs.

**Location:** `docs/DOMAIN_LANGUAGE.md` lines 334-353 (both worked examples).

**Fix:** Rewrite both examples to use `Default::default()` + field reassignment, matching the pattern that actually works for external consumers.

### 2. Rustdoc links to `docs/DOMAIN_LANGUAGE.md` are dead on docs.rs

**The crime:** I added `[Consistency model](docs/DOMAIN_LANGUAGE.md#consistency-model)` to the crate-level rustdoc in `src/lib.rs`. On docs.rs, the `docs/` directory does not exist — these are dead links.

**The evidence:** The EXISTING rustdoc already uses full GitHub URLs for external references: `[project README on GitHub](https://github.com/LarsArtmann/segment-buffer#segment-buffer)`. I ignored the established pattern and used relative filesystem paths.

**Location:** `src/lib.rs` lines 28 and 39.

**Fix:** Change to `https://github.com/LarsArtmann/segment-buffer/blob/master/docs/DOMAIN_LANGUAGE.md#consistency-model` (or link to the README, which IS rendered on docs.rs).

### 3. Unintended `flake.lock` bump swept into the change set

**The crime:** Running `scripts/verify-gate.sh` (which calls `nix flake check`) bumped `flake.lock` for rust-overlay. This dependency-update has nothing to do with the book-insights execution work and should NOT be in the same commit.

**The fix:** Revert `flake.lock` before committing, or commit it separately with a `chore:` prefix.

### 4. Unintended markdown table reformatting

**The crime:** `nix fmt` / treefmt reformatted all markdown tables in `docs/book-insights-mapping.md` (128 lines changed: column alignment) and `docs/planning/2026-07-23_15-50_book-insights-action-plan.md` (table alignment). These are legitimate formatting fixes but they're UNRELATED to my work and inflate the diff.

**The fix:** Either accept these as formatting normalization (they ARE the correct format) or revert them to keep the diff focused. They should NOT be silently mixed into a feature commit without mention.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

| #   | Issue                                                                                                                                               | Fix                                                                                                      |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| 1   | **I didn't commit incrementally.** Everything is batched in one uncommitted blob.                                                                   | Commit after each logical unit (test, docs section, gate run).                                           |
| 2   | **I didn't re-read my own code for the `#[non_exhaustive]` error.** I fixed it in the test but propagated it in the docs.                           | After hitting a compiler error, grep for the same pattern in everything I wrote that session.            |
| 3   | **I claimed "done" without committing.** The AGENTS.md session-end checklist says `git status` must appear in the same message as any "done" claim. | Run `git status` before writing any closing summary.                                                     |
| 4   | **I didn't catch the rustdoc link pattern violation.** The existing code uses full GitHub URLs; I used relative paths.                              | Before adding links to rustdoc, check how existing links in the same file are formatted.                 |
| 5   | **I let `nix fmt` side-effects bleed into my change set without noting them.**                                                                      | After running the verification gate, `git diff --stat` to see ALL changes, not just the ones I intended. |

### Technical improvements

| #   | Issue                                                                                                                                                                                                                   | Fix                                                                                                                         |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 6   | **Allocation budgets are measured on one machine.** The "machine-independent" claim is aspirational. CI might show different counts.                                                                                    | Add a comment in the test: "If this fails on your platform, re-measure and update the constant + measured value."           |
| 7   | **The concurrent test doesn't test the flush-race.** Only the delete-race has coverage.                                                                                                                                 | Write `concurrent_read_and_flush_never_corrupts` that exercises the transient-gap window.                                   |
| 8   | **The consistency model's "canonical vs concurrent" boundary is fuzzy.** A user running `append` on one thread and `read_from` on another is arguably "canonical" for a multi-producer buffer, but hits the flush race. | Sharpen the boundary: "canonical" = single-threaded or serial; "concurrent" = any overlapping calls from different threads. |

---

## f) Up to 50 Things to Get Done Next

### Critical (fix before committing)

1. **Fix the `SegmentConfig` struct-literal examples** in DOMAIN_LANGUAGE.md Tradeoffs section → use `Default::default()` + field reassignment
2. **Fix the rustdoc links** in `src/lib.rs` → use full GitHub URLs matching the existing pattern
3. **Revert `flake.lock`** → it's an unrelated dep bump from the verification gate
4. **Decide on markdown table reformatting** → keep (it's correct format) or revert (to keep diff focused)

### Should have been done this session

5. **Update FEATURES.md test count** from "Unit tests (82)" to "Unit tests (83)"
6. **Add FEATURES.md entry** for the allocation-count guard under "Testing & trust"
7. **Add FEATURES.md entry** for the concurrent-race boundary test
8. **Add CHANGELOG.md entry** for all new work (Unreleased section)
9. **Update AGENTS.md** with the two-race-window discovery in the concurrency invariant section
10. **Write the flush-race test** (`concurrent_read_and_flush_never_corrupts`)
11. **Commit** with proper message and separation

### Quality hardening

12. **Add allocation-guard run to the verify-gate script** so it's not silently skipped when someone uses `--no-loom` etc.
13. **Add the percentile extraction one-liner as a `flake.nix` app** (`nix run .#p99`) so the pre-release check is one command
14. **Cross-link the perf doc** from `docs/PERFORMANCE.md` if it exists, or from the README performance section
15. **Verify the alloc_guard test passes on the MSRV (1.86)** — the global allocator pattern might behave differently
16. **Run the alloc_guard test 10× in release** to confirm the budgets aren't tight enough to flake
17. **Consider whether the alloc_guard should also measure `delete_acked`** — it's the commit point and its allocation profile matters

### Documentation consistency

18. **Add the alloc_guard concept to CONTRIBUTING.md** testing section
19. **Verify all new DOMAIN_LANGUAGE.md anchor links** actually resolve (the section names contain spaces → anchor slugification)
20. **Check if `docs/DOMAIN_LANGUAGE.md#schema-evolution-of-t` is the correct anchor** (the `of-T` suffix might not slugify correctly)
21. **Add the Tradeoffs section to the README** "How it works" area — consumers looking for knobs will find the README first
22. **Reconcile the consistency model language** between README ("at-least-once delivery built in") and the new DOMAIN_LANGUAGE.md nuance (canonical vs concurrent)
23. **Document the allocation-guard design decision** in a planning doc — future maintainers need to know WHY budgets were chosen, not just what they are

### Testing improvements

24. **Add a property test for the consistency model**: proptest that verifies read-your-writes holds under sequential append+read
25. **Add a property test for monotonic reads**: proptest that verifies increasing start offsets never see backward movement
26. **Add a stress test combining flush + delete + read concurrently** (three-way race, not just pairwise)
27. **Consider a loom test for the flush-race** (currently only delete_acked+append is loom-proven)
28. **Add a test that `read_from` returns items in ascending seq order** under concurrent appends (not just no-corruption)
29. **Test the allocation guard under `--features encryption`** with a cipher configured (the encrypt path has different allocation characteristics)

### Future features (correctly deferred, but should be tracked)

30. **Envelope v2 cipher auto-detection** — the consistency model docs reference "which cipher was used" as a limitation
31. **Streaming CBOR early-stop at limit** — the perf doc explicitly calls this out as the highest-impact read-path optimization
32. **Compression-algorithm negotiation** — the Tradeoffs section lists it as the one genuine gap in tradeable knobs
33. **Health check implementation** — un-defer when a concrete consumer needs more than `stats() + trial append`
34. **Async I/O** — on ROADMAP, untouched, correctly deferred
35. **Second SegmentStore impl** — on ROADMAP, untouched, correctly deferred

### Docs-health sweep (after committing)

36. **Run the full `scripts/verify-gate.sh` including supply-chain** (`cargo audit` + `cargo deny`) — I skipped this in the session
37. **Run `lychee` on all new docs** — the verify-gate ran it but I didn't verify the output targeted my new files
38. **Run the docs-health skill** to check for drift between the new DOMAIN_LANGUAGE.md sections and the rest of the doc suite
39. **Check if the `docs/book-insights-mapping.md` should be moved** to `docs/planning/` (it's a point-in-time analysis, not a living doc)
40. **Verify CI is green** (`gh run list --limit 4`) before any further work — AGENTS.md rule 10

### Architectural reflection

41. **Consider whether the consistency model belongs in DOMAIN_LANGUAGE.md or a dedicated `docs/CONSISTENCY_MODEL.md`** — it's 80 lines and growing; DOMAIN_LANGUAGE.md is getting long
42. **Evaluate whether the allocation guard should become a bench assertion** (criterion + iai) rather than a standalone test — integration with the existing bench infrastructure
43. **Consider whether the Tradeoffs section should be a decision tree** (flowchart) rather than a table — consumers have multi-dimensional needs
44. **Reflect on whether `read_from` should gain a `retry-on-NotFound` mode** — the docs say "retry", but the crate provides no helper for it
45. **Consider whether the flush-race window can be closed** without holding the lock across I/O — e.g., re-check `unflushed` in Phase 2 even if Phase 1 returned items

### Cleanup

46. **Decide whether `docs/book-insights-mapping.md` is living or point-in-time** — if point-in-time, add a "Captured:" date and a "may be stale" note
47. **Add the action plan execution log** to the status report cross-references in AGENTS.md
48. **Remove or annotate the saga-pattern references** in the action plan doc (T5 was dropped; the plan still mentions it in the body)
49. **Verify the mermaid graph in the action plan** is updated to reflect T5 being dropped (it still shows T5)
50. **Add a "Last verified" date** to the percentile-latency perf doc — criterion output rots fast

---

## g) Questions I CANNOT Answer Myself

### Q1: Should the `flake.lock` bump be committed separately, reverted, or ignored?

The `nix flake check` in the verification gate bumped `rust-overlay` from `rev 47759fa` to `rev 19a19f3`. This is an automated dependency update unrelated to my work. Options: (a) revert it and let the weekly `update-flake-lock.yml` workflow handle it, (b) commit it separately as `chore: bump rust-overlay in flake.lock`, (c) include it in the main commit. I cannot decide this because it depends on your policy on whether verification-gate side-effects should be committed.

### Q2: Should the markdown table reformatting (128 lines in book-insights-mapping.md) be kept or reverted?

`nix fmt` / treefmt normalized all markdown table column alignment across two files I didn't intend to modify (`docs/book-insights-mapping.md` and the action plan doc). The formatting IS correct (aligned tables are the treefmt standard), but it inflates the diff and mixes formatting with content. I cannot decide whether you prefer "all formatting in one commit" vs "keep diffs focused on intent."

### Q3: Should I commit this work now (after fixing the three fuckups in §d) or wait for the flush-race test and FEATURES.md/CHANGELOG.md/AGENTS.md updates?

The core work is done and verified, but the bookkeeping (FEATURES.md count, CHANGELOG, AGENTS.md learnings) and the missing flush-race test are unfinished. I cannot decide whether you prefer "commit what's verified now, iterate after" vs "do everything, then commit once."
