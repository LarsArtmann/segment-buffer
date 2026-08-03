# Status Report: Panic-Free API + Three Deferred Design Decisions Resolved

**Date:** 2026-08-04 01:12
**Session scope:** Resolve three deferred design decisions from `paste_1.txt`, implement the chosen paths end-to-end, verify, and document.

---

## a) FULLY DONE

### 1. Panic-free public API via root-cause deadlock elimination (SHIPPED)

**The problem:** `for_each_from` was the _sole_ panic path in the library. It held the buffer mutex across the user callback (Phase 2 in-memory iteration), so re-entering the buffer from the callback would deadlock (`parking_lot::Mutex` is not reentrant). A panic guard (`assert_not_reentered`) converted the silent deadlock into a loud panic.

**The fix:** Eliminated the deadlock at its root. `for_each_from` Phase 2 now snapshots the in-memory pending window under the lock, then releases the lock _before_ invoking the callback. Re-entrant calls (`append`, `stats`, `delete_acked`, ...) from inside a callback are now safe — they acquire the mutex normally because `for_each_from` already released it.

**Deleted infrastructure:**

- `assert_not_reentered()` method + its 10 call sites
- `iteration_in_progress: AtomicBool` field + constructor init
- `IterationGuard` RAII struct + `Drop` impl
- All 11 `# Panics` doc sections across public methods
- All dead `#[track_caller]` attributes (11 removed)
- `#[allow(clippy::panic)]` exception

**Added:** Explicit `drop(inner)` after the snapshot to satisfy clippy `significant_drop_tightening` (minimal lock hold).

**Tradeoff:** The in-memory tail is now cloned once (bounded by `limit`, not the whole backlog). The old "~21x faster than read_from" claim is no longer true — both paths are now roughly equal (~23 us at 1k items). All docs updated with honest measured numbers.

**Files changed:** `src/lib.rs` (structural), `src/tests.rs` (tests rewritten)

### 2. Re-entrancy tests rewritten (3 tests replace 2)

Old tests asserted the panic behavior. New tests assert the panic-free behavior:

- `for_each_from_allows_reentry_without_deadlock` — re-entrant reads (`pending_count`, `latest_sequence`, `stats`) succeed from inside callback
- `for_each_from_allows_reentrant_mutation` — re-entrant `append` from inside callback lands in the tail (visited count stays at snapshot size)
- `for_each_from_usable_after_panicking_callback` — buffer is usable after a panicking callback (mutex was never held across it)

### 3. Documentation updated across all living docs

| File                      | What changed                                                                                                                                                                                                                                             |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`               | New "Guarantees" section (panic-free, at-least-once, single-process, crash recovery). Updated "Unreleased" note.                                                                                                                                         |
| `src/lib.rs`              | `for_each_from` doc: "Re-entrancy" section rewritten, perf table updated with measured numbers. `iter_from` doc: "Re-entrancy" section rewritten. Compressor mutex comment fixed (stale "re-entrancy guard" reference). All `# Panics` sections removed. |
| `AGENTS.md`               | Concurrency invariant: added for_each_back mutex-release guarantee. Lint architecture: noted zero panic paths.                                                                                                                                           |
| `CHANGELOG.md`            | New `[Unreleased]` Changed entry for the root-cause fix. Removed "the only panic is the documented for_each_from re-entrancy guard" qualifier.                                                                                                           |
| `FEATURES.md`             | `for_each_from` row: updated from "zero-clone / 21x" to honest "on par" description. Re-entrancy guard row replaced with "Panic-free re-entrancy" row.                                                                                                   |
| `docs/DOMAIN_LANGUAGE.md` | `for_each_from` entry rewritten. Perf trade-off table row updated. `iter_from` lending references fixed.                                                                                                                                                 |
| `docs/PERFORMANCE.md`     | Section 4 fully rewritten (callback vs owned Vec, measured numbers, no more "zero allocation" / "21x faster"). Bench description updated. Ratio claims section updated.                                                                                  |
| `ROADMAP.md`              | Panic-free guarantee design question marked resolved.                                                                                                                                                                                                    |
| `TODO_LIST.md`            | All three deferred decisions resolved (see below).                                                                                                                                                                                                       |

### 4. Three deferred design decisions RESOLVED

| Decision                      | Verdict               | Rationale                                                                                                                                                                                                                                           |
| ----------------------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Health-check primitive        | **DEFER**             | All 3 candidate designs are Verschlimmbessern (redundant, disk-harmful, platform dependency). Canonical health check: `stats()` + explicit `flush()`. Also corrected stale "Drop panics on lock tampering" claim (it's best-effort, doesn't panic). |
| Document panic-free guarantee | **SHIPPED**           | Root-cause fix eliminated the only panic. Documented as a quality bar in README (not a load-bearing API contract).                                                                                                                                  |
| mtime scan-cache gap          | **FORMALLY ACCEPTED** | Single-process invariant already forbids external dir mutation. The mtime guard is defense-in-depth against contract violations, not a primary guarantee.                                                                                           |

---

## b) PARTIALLY DONE

### Perf docs benchmark update — PARTIALLY DONE

The `docs/PERFORMANCE.md` claims are updated with indicative numbers from a single reduced-sample benchmark run (`--sample-size 10`). A full benchmark run with criterion's default sample size was not performed. The numbers are indicative, not publication-grade — which the doc itself states.

### CI green verification — BLOCKED (not my code)

The most recent CI run (`30855036471`, dependabot nix commit from 2026-08-03) is red on the MSRV 1.86 job. I verified locally via `nix develop .#msrv -c cargo clippy --all-targets -- -D warnings -A clippy::pedantic` that current code passes (exit 0). The fix (`#[allow]` for `missing_const_for_fn` and `needless_collect`) is in concurrent commits, not mine. **26 commits are unpushed** — CI has not validated the combined work.

---

## c) NOT STARTED

- **Push to remote** — 26 commits ahead of `origin/master`. Per AGENTS.md rule 11 ("NEVER PUSH TO REMOTE unless explicitly asked"), this is blocked on user instruction.
- **Release tag** — No release was requested or attempted.
- **Property test for for_each_from under concurrent flush** — `TODO_LIST.md` line 92 mentions this; it was not in scope for this session.

---

## d) TOTALLY FUCKED UP

### Nothing was irreversibly damaged. But these were mistakes:

1. **Missed PERFORMANCE.md and ROADMAP.md on first doc sweep.** I updated `src/lib.rs`, `FEATURES.md`, and `docs/DOMAIN_LANGUAGE.md` but missed `docs/PERFORMANCE.md` (3 stale "~21x faster" / "zero allocation" / "re-entry panics" claims) and `ROADMAP.md` (unresolved design question). Caught on a second sweep prompted by the user's "what did you forget?" — **should have done a comprehensive `rg` sweep across ALL docs in one pass before declaring docs done.**

2. **Missed two stale references in DOMAIN_LANGUAGE.md** ("zero-copy lending alternative" at line 108, "lending iterator stays" at line 245) — these were in different sections of the same file I had already edited. Caught on third sweep. **Should have used `rg -n 'lending\|zero.copy' docs/DOMAIN_LANGUAGE.md` specifically after editing that file.**

3. **The auto-commit daemon caused repeated edit races.** My `edit`/`multiedit` calls failed 3+ times because the daemon touched the file between my `view` and `edit` calls. I adapted mid-session by switching to Python `open/read/replace/write` for atomic edits — **should have used this approach from the start** since the AGENTS.md explicitly says "An auto-git commit daemon runs continuously."

4. **Doc warning in `segment_size_stats`** (concurrent commit `fead239`) — `cargo doc` warns about an intra-doc link to private `Self::scan_segments`. Not my code, not my responsibility per lint rules ("Fix issues in files you changed"). But I noticed it and did not flag it clearly until now.

---

## e) WHAT WE SHOULD IMPROVE

1. **for_each_from performance regression.** The root-cause fix (snapshot + release lock before callback) inherently clones the in-memory window. The old zero-clone lending path was ~21x faster than `read_from`. Now they're roughly equal. A future improvement could explore: (a) `Arc<Vec<T>>` snapshot (one atomic refcount bump instead of per-item clone), (b) a GAT-based lending iterator (stable Rust may support this soon), or (c) accepting the tradeoff as the cost of panic-freeness.

2. **Comprehensive doc sweep as a single gate step.** The pattern of "edit docs → user asks what did you forget → find more stale refs" happened 3 times. A single `rg -rn '<old-claim>' docs/ src/ *.md` pass before declaring docs done would have caught everything.

3. **Use atomic file writes when the daemon is active.** Python `open/read/replace/write` is race-free against the auto-commit daemon. The `edit` tool is not. This should be the default approach in repos with auto-commit enabled.

4. **The loom test `delete_acked_idempotent_under_concurrent_append` took 218 seconds** (over the 60s background threshold). This is pre-existing but means the loom gate takes ~4 minutes. Worth investigating whether the schedule enumeration can be pruned.

5. **FEATURES.md test count says "(109)" but the actual count is now 109** — the concurrent `segment_size_stats` commit updated it. This was correctly NOT touched by me. But it shows how multiple agents editing the same files creates merge complexity.

---

## f) Up to 50 things to get done next

### High priority (blocks release)

1. Push the 26 unpushed commits to `origin/master` and verify CI goes green
2. Verify the MSRV 1.86 clippy errors are truly resolved by CI (not just local)
3. Fix the `cargo doc` warning in `segment_size_stats` (private intra-doc link) — even though it's not my code, it's a doc-gate issue
4. Run the full `scripts/verify-gate.sh` (14 gates) before any release tag
5. Check `gh run list --limit 4` after push to confirm CI green (Rule 9/10)

### Performance investigation

6. Run full `cargo bench --bench bench_read_vs_for_each` with default sample size for publication-grade numbers
7. Explore `Arc<Vec<T>>` snapshot for for_each_from (refcount bump vs per-item clone)
8. Benchmark the clone overhead at various `limit` values (100, 1k, 10k, 100k)
9. Consider whether the `for_each_from` Phase 1 (on-disk segments) also needs the snapshot treatment (it already decodes to a local Vec, so it's fine — but verify)
10. Update `docs/perf/` snapshots with new for_each_from numbers

### Correctness hardening

11. Add a property test for `for_each_from` under concurrent `flush` (TODO_LIST line 92)
12. Add a property test proving `for_each_from` and `read_from` return the same items after the snapshot change
13. Add a loom test for `for_each_from` under concurrent mutation (the snapshot + release pattern is new)
14. Verify re-entrant `for_each_from` inside `for_each_from` is safe (nested iteration)
15. Add a stress test for re-entrant `append` under heavy load (does the cloned window cause memory pressure?)

### Documentation polish

16. Update `CONTRIBUTING.md` if it references the re-entrancy guard or panics
17. Run `lychee` markdown link check locally before push (CI gate)
18. Check `docs/planning/` and `docs/status/archived/` for the `update-old-docs` skill (non-destructive annotation of stale ~21x claims in historical docs)
19. Update the `docs/status/2026-08-02_05-03_namtao-rust-learnings-and-strict-lint-adoption.md` status report (referenced by ROADMAP as source of the panic-free design question — now resolved)
20. Add a `# Guarantees` section to `src/lib.rs` crate docs (mirrors README)

### API ergonomics

21. Consider whether `for_each_from` should return `Result<usize>` or whether the callback can now be infallible (no more deadlock risk) — probably keep Result for I/O errors
22. Consider deprecating the `FlushPolicy::Manual` requirement for concurrency tests (Rule 7) — the snapshot pattern may make `Batch(4)` safe for stress tests now
23. Evaluate whether `iter_from` can become re-entrancy-safe in the same way (it already materializes eagerly via `read_from`, so it's already safe — verify and document)

### Release preparation

24. Draft CHANGELOG `[Unreleased]` → versioned section when ready to release
25. Verify `Cargo.toml` version bump is needed (behavioral change in for_each_from, but no API signature change — is this a minor or patch?)
26. Update `html_root_url` in `src/lib.rs` if version bumps
27. Prepare GitHub release notes draft (before tag push, per release runbook step 7)
28. Run `scripts/check-msrv.sh` to verify MSRV consistency across all surfaces

### Cleanup

29. Audit all `#[allow(clippy::*)]` attributes — are any now unnecessary after the guard removal?
30. Check if `#[track_caller]` was removed from ALL methods (it existed for the panic guard; now that the guard is gone, any remaining `#[track_caller]` is dead)
31. Verify `examples/` still compile correctly (they call getters that lost `#[track_caller]`)
32. Run `cargo bench --bench bench_stats` to verify stats() didn't regress (lock was already infallible, but verify)
33. Check if `alloc_guard.rs` integration test needs updating
34. Sweep `fuzz/` targets for any references to re-entrancy or panics

### Design future

35. Explore whether a `TryAppend` / `try_append` variant is needed for callers who want to detect re-entrancy at compile time instead of allowing it
36. Consider a `for_each_from_owned` variant that consumes items instead of borrowing (useful for drain-and-delete patterns)
37. Document the panic-free guarantee more formally (maybe a `# Panics` section at the crate level saying "This crate has zero panic paths")
38. Consider adding `#[cfg(debug_assertions)]` assertions for the snapshot invariant (visited count <= limit)
39. Explore whether the scan-cache mtime gap can be closed with a generation counter instead of mtime (avoids the filesystem dependency entirely)
40. Evaluate whether `health()` is needed for k8s readiness probes (the user said "proactive" — may want to revisit)

### Process

41. Add a "comprehensive stale-reference sweep" step to the docs-health skill
42. Document the "use Python for atomic edits when daemon is active" pattern in AGENTS.md
43. Consider whether the auto-commit daemon should be paused during active editing sessions
44. Review whether the concurrent `segment_size_stats` work needs reconciliation with my changes before push
45. Verify `Cargo.lock` is still committed after all changes

### Testing infrastructure

46. Add the new for_each_from tests to the loom coverage list in AGENTS.md
47. Update FEATURES.md test counts after concurrent work settled (109 unit + 25 property currently, but concurrent commits may change this)
48. Consider a `#[test]` that asserts zero `panic!` calls exist in library code (grep-based guard)
49. Add a CI job that runs `cargo bench --bench bench_read_vs_for_each` and fails on >10% regression vs a baseline
50. Document the for_each_from snapshot pattern in AGENTS.md as a design precedent for future callback APIs

---

## g) Questions I CANNOT figure out myself

### 1. Should I push the 26 unpushed commits now?

26 commits are ahead of `origin/master`. CI was last green on older code. My local MSRV 1.86 check passes, but the combined work (mine + concurrent `segment_size_stats` + other concurrent commits) has never been validated by CI. AGENTS.md Rule 10 says "CI-red is a stop-work condition" but Rule 11 says "NEVER PUSH unless explicitly asked." These conflict when the only way to verify CI is to push. **Do you want me to push?**

### 2. Is the for_each_from performance regression acceptable, or should I explore alternatives?

The root-cause fix clones the in-memory window (bounded by `limit`). This eliminated the panic but regressed for_each_from from ~21x faster than read_from to roughly equal. Alternatives exist (`Arc<Vec<T>>` snapshot, GAT-based lending iterator) but each has tradeoffs. **Is "roughly equal to read_from" acceptable as the permanent cost of panic-freeness, or should I investigate Arc-snapshot or other zero-clone approaches?**

### 3. Should I fix the concurrent `segment_size_stats` doc warning?

`cargo doc` produces a warning: `segment_size_stats` links to private `Self::scan_segments`. This is in commit `fead239` (not mine). Per lint rules I should only fix files I changed. But it's a doc-gate issue that will show as a warning on every `cargo doc`. **Do you want me to fix it, or leave it for whoever authored that commit?**

---

## Verification gate (this session)

| Gate                | Command                                                                               | Result                                                     |
| ------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Format              | `cargo fmt --all -- --check`                                                          | PASS                                                       |
| Clippy (default)    | `cargo clippy --all-targets -- -D warnings`                                           | PASS                                                       |
| Clippy (encryption) | `cargo clippy --all-targets --features encryption -- -D warnings`                     | PASS                                                       |
| Tests               | `cargo test --no-fail-fast --features encryption`                                     | 171 pass (131 unit + 1 alloc_guard + 39 doctest)           |
| Loom                | `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`             | 12 pass (218s)                                             |
| Docs                | `cargo doc --no-deps --features encryption`                                           | exit 0 (1 warning in concurrent code)                      |
| MSRV 1.86           | `nix develop .#msrv -c cargo clippy --all-targets -- -D warnings -A clippy::pedantic` | exit 0                                                     |
| CI (remote)         | `gh run list --limit 4`                                                               | RED on MSRV 1.86 (pre-existing, fixed in unpushed commits) |

---

## Session metrics

- **Test count change:** 2 old tests → 3 new tests (net +1 in tests.rs; concurrent work added more)
- **Lines deleted:** ~80 (guard infrastructure + # Panics sections + IterationGuard)
- **Lines added:** ~50 (snapshot logic + new tests + doc updates)
- **Files touched:** 9 (`src/lib.rs`, `src/tests.rs`, `README.md`, `AGENTS.md`, `CHANGELOG.md`, `FEATURES.md`, `TODO_LIST.md`, `ROADMAP.md`, `docs/DOMAIN_LANGUAGE.md`, `docs/PERFORMANCE.md`)
- **Stale-reference sweeps needed:** 3 (should have been 1)

---

## Resolution (2026-08-04)

The panic-free API is shipped. All three deferred design decisions are resolved
(panic-free SHIPPED, health-check DEFER, mtime gap FORMALLY ACCEPTED).

| Item | Claim in report | Resolution | Commit | Release |
| ---- | --------------- | ---------- | ------ | ------- |
| b.2  | CI green verification — blocked | RESOLVED: CI is now green on master (MSRV lint issue diagnosed and fixed in concurrent commits) | — | — |
| d.3  | cargo doc warning in segment_size_stats | DONE: private intra-doc link to `Self::scan_segments` converted to plain text | `01-14` session | unreleased |
| f.3  | Fix cargo doc warning (private intra-doc link) | DONE: fixed by the 01-14 session | `01-14` session | unreleased |
| f.4  | Run full verify-gate.sh | STILL OPEN — no session has run the full gate end-to-end | — | — |
| f.5  | Check gh run list after push | RESOLVED: CI is green on master | — | — |
| f.6  | Full benchmark run for publication-grade numbers | STILL OPEN — indicative numbers are in docs/perf but default-sample-size run not done | — | — |
| g.1  | Push the 26 unpushed commits? | RESOLVED: commits pushed; CI green | — | — |

**Still open:** f.4 (run verify-gate.sh end-to-end), f.6 (full benchmark run), g.2 (for_each_from perf regression acceptable? — the tradeoff is documented; a future `Arc<Vec<T>>` snapshot is a ROADMAP-level investigation).
