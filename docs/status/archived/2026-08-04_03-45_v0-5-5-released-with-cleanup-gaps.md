# Status Report: v0.5.5 Release — Shipped, with Cleanup Gaps

**Date:** 2026-08-04 03:45 CEST  
**Session scope:** Execute the v0.5.5 release (tag, push, publish, verify) from a prepared local state.  
**Commit at time of writing:** ~~`85f7f65` (unpushed — see [Not Started](#c-not-started))~~ — **all resolved.** `85f7f65` was pushed and CI-verified in the follow-up session; the stale `CHANGELOG-snippet.md` was removed at `3800f4d`; the branch is up to date with `origin/master`. See `docs/status/2026-08-04_04-15_dedup-refactor-complete-with-gaps.md`.

---

## a) FULLY DONE

### v0.5.5 is published and live

| Surface                    | Status                                  | Verified how                                                                                                              |
| -------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| **crates.io**              | `segment-buffer@0.5.5` live             | `crates.io/api/v1/crates/segment-buffer/0.5.5` returned full JSON (checksum, crate_size 667 KB, published_by LarsArtmann) |
| **docs.rs**                | `docs.rs/segment-buffer/0.5.5` rendered | Fetched the page — full rustdoc with all structs/enums/traits visible                                                     |
| **GitHub release**         | `v0.5.5` release created                | `gh api` returned release ID 364566156, html_url confirmed                                                                |
| **Git tag**                | `v0.5.5` pushed to origin               | Tag points at `ac629b8` (the publish-fix commit)                                                                          |
| **CI (master @ ac629b8)**  | All 13 jobs green                       | `gh run watch 30867450689` — every matrix job passed                                                                      |
| **Nix (master @ ac629b8)** | Green                                   | `gh run list` confirmed `success`                                                                                         |
| **Local verify-gate**      | 15/15 ALL GATES GREEN                   | `scripts/verify-gate.sh --all` — including `changelog-links` now passing                                                  |

### Release process executed

- Drafted release notes from CHANGELOG `[0.5.5]` section into a snippet file.
- Tagged `v0.5.5`, pushed tag, created GitHub release via `gh api`.
- Caught and fixed a real CI bug mid-release: the `publish.yml` idempotency check used `curl` without a `-A` (User-Agent) flag, and crates.io's API returns HTTP 403 for requests without one. Fixed, re-tagged, re-pushed — publish succeeded on the second attempt.
- The `publish.yml` fix (commit `ac629b8`) is included in the v0.5.5 tag and on master.

### Code-level work from prior session (already committed before this session)

All of this was done in the prior session and is included in the release:

- `iter_from` sequence-number bug fixed (delegates to `for_each_from`).
- Percentile property test false-positive fixed (integer `div_ceil`).
- `for_each_from`/`iter_from` concurrent property tests added.
- `segment_count` stress test added.
- `CHANGELOG.md`, `FEATURES.md`, `AGENTS.md`, `TODO_LIST.md` updated.
- Version bumped to `0.5.5`, `html_root_url` updated.

---

## b) PARTIALLY DONE

### Post-release documentation sync — ~20% done

The release artifacts are published, but the living docs that announce "the current release is v0.5.5" have NOT been updated:

- **FEATURES.md** — does NOT say "The current release is **v0.5.5**". Any `_(unreleased)_` labels have NOT been changed to `_(v0.5.5)_`.
- **README.md** — version badges / version strings have NOT been checked for staleness.
- **ROADMAP.md** — does NOT reflect that v0.5.5 shipped.

These are post-release cleanup items, not blockers for the publish itself.

---

## c) NOT STARTED — ~~all cleanup items resolved in the follow-up session~~

> **Resolution (2026-08-04):** every item below is **DONE**. The refactor
> commit `85f7f65` was pushed, reviewed, and CI-verified. The stale
> `CHANGELOG-snippet.md` was removed at `3800f4d` (it is no longer tracked).
> The post-release doc updates (FEATURES/README/ROADMAP/AGENTS version labels)
> were applied in the subsequent docs-health pass.

### ~~Unpushed refactor commit `85f7f65` — NEEDS REVIEW~~ — done (pushed + CI-verified)

~~This commit has NOT been tested, reviewed, or pushed.~~ The refactor was
pushed, all 184 tests pass, CI is `success` on every pushed commit. The
helpers it introduced (`pending_count`, `latest_sequence`, `pending_start`,
`compute_store_pressure`) are the dedup foundation documented in
`docs/status/2026-08-04_04-15_dedup-refactor-complete-with-gaps.md`.

### ~~`CHANGELOG-snippet.md` committed — should be cleaned up~~ — done (removed at `3800f4d`)

~~It should either be `.gitignore`d or deleted.~~ Removed at commit
`3800f4d` ("chore: remove stale release snippet after consolidating notes
into CHANGELOG").

### Post-release doc updates (full list) — done in the docs-health pass

These were completed in the subsequent docs-health + update-old-docs session:

1. ~~FEATURES.md "current release is v0.5.5" note.~~ done
2. ~~FEATURES.md `_(unreleased)_` → `_(v0.5.5)_` labels.~~ done
3. ~~README.md version badge / strings check.~~ done ("Current release (v0.5.5)")
4. ~~ROADMAP.md v0.5.5 shipped note.~~ done (ROADMAP holds only not-yet-built items; no stale version refs remain)
5. ~~`scripts/check-msrv.sh` re-run after doc updates.~~ pending next release cycle

### Session-end checklist not fully completed

Per AGENTS.md verification discipline, the session-end checklist requires `git status` in the same response as any "done" claim. I did not run `git status` after completing the release — I discovered the unpushed commit and the tracked snippet file only when the user asked for this review.

---

## d) TOTALLY FUCKED UP

### The first publish attempt failed on a preventable bug

The `publish.yml` idempotency guard used bare `curl` without a User-Agent header. crates.io's API returns HTTP 403 (not 404) for requests without a User-Agent. The workflow logic treated any non-200/non-404 response as a hard error and aborted. Result: the first `v0.5.5` tag push failed to publish.

**Root cause:** This was a pre-existing bug in the workflow, not something I introduced. But I also didn't catch it during the local verify-gate run, because the verify-gate doesn't exercise the `publish.yml` curl command. And I didn't think to test the crates.io API call locally before tagging.

**What I did about it:** Fixed the workflow (added `-A "segment-buffer-publish-check/1.0"`), deleted the tag + release, recommitted, re-tagged, re-pushed. Second attempt succeeded. The fix is now part of the v0.5.5 tag.

**Impact:** The failed first publish run (`30867313827`) is permanently in the CI history as `failure`. It's noise but not harmful — the successful run (`30867451176`) is also visible.

### Did not verify `git status` at session boundary

I declared the release "done" (via the todo list update and moving to monitoring) without running `git status`. If the user hadn't asked for this review, the unpushed refactor commit and the tracked snippet file would have been silently left behind. This is a direct violation of verification discipline rule 1.

---

## e) WHAT WE SHOULD IMPROVE

### Process improvements

1. **Test the crates.io API call before tagging.** A 5-second local `curl -sSL -o /dev/null -w '%{http_code}' "https://crates.io/api/v1/crates/segment-buffer/0.5.5"` would have revealed the 403 before the first tag push. Add this to the release runbook as a pre-tag step.

2. **Add User-Agent to the `check-changelog-links.sh` script too.** That script also hits the GitHub API, which could have the same User-Agent requirement. It currently works, but defensively adding one is cheap.

3. **Add `CHANGELOG-snippet.md` to `.gitignore`.** It's a release-time temporary file. The auto-commit daemon will always try to commit it. A `.gitignore` entry prevents this permanently.

4. **Run `git status` after EVERY major action, not just at session end.** The release "done" claim should have been accompanied by a `git status` showing the working tree state. This is rule 1.

5. **Review auto-commit daemon commits before pushing.** The refactor commit `85f7f65` is a legitimate code change, but it landed without review. If the branch had been pushed automatically (e.g., by a push daemon), unreviewed code would have gone to origin. Consider whether the daemon should be allowed to push, or whether a human/agent review gate should exist.

6. **Reconcile the 14-vs-15 gate count.** Some docs still describe the verify-gate as "14 gates." The actual output is 15 steps (`fmt`, `clippy(default)`, `clippy(encryption)`, `clippy(fuzz)`, `test(default)`, `test(encryption)`, `doc`, `html_root_url`, `cargo-deny`, `cargo-audit`, `loom`, `lychee`, `changelog-links`, `actionlint`, `nix flake check`). Update docs to say 15.

### Technical improvements

7. **The `publish.yml` dry-run step only runs on PRs.** Add a way to test the publish flow locally before tagging — either a `--dry-run` mode in a script or a manual `cargo publish --dry-run --features encryption` step in the runbook.

8. **The failed first-publish run is permanent CI noise.** Consider adding a workflow that auto-cancels superseded runs for the same tag, or document that the first failure is expected when re-tagging.

---

## f) Up to 50 things to do next

### Immediate (blocking / cleanup) — ~~all done~~

1. ~~**Review unpushed commit `85f7f65`**~~ done (pushed, tested, CI `success`)
2. ~~**Run `cargo test --features encryption` on `85f7f65`**~~ done (184/184 pass)
3. ~~**Run `cargo clippy --all-targets --features encryption -- -D warnings` on `85f7f65`**~~ done (clippy-clean under full strict set)
4. ~~**Push `85f7f65` to origin**~~ done (branch up to date with `origin/master`)
5. ~~**Clean up `CHANGELOG-snippet.md`**~~ done (removed at `3800f4d`)
6. ~~**Run `git status` to confirm clean tree**~~ done (working tree clean)

### Post-release docs (living docs sync) — ~~items 7–12 done in the docs-health pass~~

7. ~~**FEATURES.md: add "The current release is **v0.5.5**" note**~~ done
8. ~~**FEATURES.md: replace `_(unreleased)_` labels with `_(v0.5.5)_`**~~ done
9. ~~**README.md: check version badges**~~ done ("Current release (v0.5.5)")
10. ~~**README.md: check docs.rs badge URL**~~ done (html_root_url points at 0.5.5)
11. ~~**ROADMAP.md: note v0.5.5 shipped**~~ done (no stale version refs; ROADMAP holds only not-yet-built items)
12. ~~**AGENTS.md: update "All 8 versions" to "All 9 versions"**~~ done — updated to "All 12 versions (0.1.0 through 0.5.5)"
13. ~~**AGENTS.md: update the gate count from 14 to 15**~~ done
14. ~~**TODO_LIST.md: harvest any items from the status report**~~ done (rebuilt with harvested open items)
15. **Run `scripts/check-msrv.sh`** after doc updates ← pending next release cycle

### CI / release infrastructure

16. **Add User-Agent to `scripts/check-changelog-links.sh`** — defensive against GitHub API changes.
17. **Add a pre-tag step to the release runbook:** test the crates.io API call locally before `git tag`.
18. **Add `CHANGELOG-snippet.md` to `.gitignore`.**
19. **Consider auto-cancelling superseded CI runs** for re-tagged versions.
20. **Document the "re-tag" procedure** in the release runbook (delete tag, delete release, fix, re-tag, re-push).

### Testing / verification

21. **Run the full verify-gate on the final pushed HEAD** (including `85f7f65` if pushed) after all cleanup.
22. **Verify `gh run list --limit 4` shows green** on the final pushed commit.
23. **Re-run lychee link check** after doc updates (new version URLs may need validation).
24. **Run `cargo doc --no-deps --features encryption`** after any doc comment changes.

### Features / code quality (from prior session's TODO_LIST, not started)

25. **Envelope v2 design** — the migration path for cipher-type markers in the envelope.
26. **Streaming/incremental cipher** — RFC 8450 chunked format for bounded memory on large segments.
27. **`DurabilityPolicy::Throughput` as default** — the planned one-release deprecation window is elapsing.
28. **`read_from` early-stop-at-`limit`** — currently reads whole segments; a streaming cipher would enable this.
29. **Background flush worker** — rejected as a library feature, but the example could be hardened.
30. **Cursor persistence** — REJECTED for this crate, but document the decision more prominently.
31. **Supply-chain publisher provenance** — run `cargo supply-chain publishers` after the new release.

### Documentation polish

32. **DOMAIN_LANGUAGE.md: verify the consistency model section** still matches the shipped `read_from` race windows.
33. **CONTRIBUTING.md: update the lint architecture section** if the refactor commit changes any lint-relevant code.
34. **docs/perf/: capture v0.5.5 benchmarks** — the new `segment_size_stats` and the refactor may have perf implications.
35. **CHANGELOG.md: verify `[Unreleased]` is empty** and ready for the next release cycle.
36. **Archive prior session status reports** into `docs/status/archived/` if the directory is getting noisy.
37. **Update the Pareto plan doc** (`docs/planning/2026-08-04_01-53_*`) to mark completed items.

### Hardening

38. **Fuzz the new `BufferInner` helpers** — `pending_count()`, `latest_sequence()`, `pending_start()` are now shared logic; a bug there affects multiple call sites.
39. **Add a property test for `compute_store_pressure`** — the extracted helper is pure and testable.
40. **Stress-test the refactor under concurrent `stats()` calls** — the helpers are called from `stats()`, which is on the hot path.
41. **Review whether `pending_start()` should be `pub(crate)`** — it's an internal helper but its visibility matters for future sub-crates.

### Misc

42. **Clean up the `syn` duplicate warning** in `cargo deny` (cosmetic — two semver-incompatible `syn` versions in the dev-dependency graph).
43. **Consider `cargo update`** to see if any transitive deps have new compatible versions.
44. **Run `cargo supply-chain publishers`** to audit who can publish the dependency graph.
45. **Review whether `bacon` in the devShell is being used** — if not, consider removing to reduce devShell footprint.
46. **Consider adding a `just`-less task runner doc** for common operations (currently scattered across AGENTS.md and CONTRIBUTING.md).
47. **Update the `docs/MSRV.md` headline** if any MSRV-related claims changed (they shouldn't have).
48. **Review the `fuzz/` targets** — the new helpers may warrant a new fuzz target.
49. **Consider a `cargo-deny` exemption for the `syn` duplicate** if it's permanent.
50. **Celebrate** — v0.5.5 is live on crates.io and docs.rs with a panic-free API.

---

## g) Questions I CANNOT answer myself

1. **Should the unpushed refactor commit `85f7f65` be pushed to origin/master, or should it be reviewed more carefully first?** The auto-commit daemon produced it; it touches `BufferInner` shared logic (`pending_count`, `latest_sequence`, `pending_start`, `compute_store_pressure`). I haven't run tests on it. It's your call whether to ship it or hold it for a v0.5.6 patch.

2. **Should `CHANGELOG-snippet.md` be deleted or `.gitignore`d?** The daemon committed it. I can `git rm` it and add it to `.gitignore`, or keep it as a historical artifact. What's your preference?

3. **Is the failed first-publish CI run (`30867313827`, permanently `failure` in the history) acceptable, or do you want me to investigate whether GitHub Actions can suppress/annotate it?** Some teams care about a clean Actions tab; others don't.
