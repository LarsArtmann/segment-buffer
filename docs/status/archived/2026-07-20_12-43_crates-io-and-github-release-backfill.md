# Status: crates.io + GitHub release backfill (all historical versions synced)

**Date:** 2026-07-20 12:43 CEST
**Session scope:** Ensure ALL historical versions exist on BOTH crates.io and GitHub releases.
**Outcome:** All 8 versions (0.1.0 → 0.5.1) now present on both surfaces. Verified consumable.
**Honesty grade:** B+. Core task succeeded. Process had three warts (documented below).

---

## a) FULLY DONE

1. **Inventoried all three sources of version truth.** Compared git tags (8), CHANGELOG headers (8), Cargo.toml version (0.5.1), crates.io API (`num_versions`), and GitHub releases via `gh release list`. Found 4 gaps total: 3 crates.io versions (0.2.0, 0.3.0, 0.4.0) + 1 GitHub release (v0.1.0).
2. **Published v0.2.0 to crates.io.** Dry-run passed (compiled + packaged cleanly). Published from a `git worktree` at the `v0.2.0` tag so the tag's `Cargo.toml` version was used. Confirmed via crates.io API: `num_versions` went 5 → 6.
3. **Published v0.3.0 to crates.io.** Same worktree procedure. Dry-run passed. Confirmed: `num_versions` 6 → 7.
4. **Published v0.4.0 to crates.io.** Same procedure. Dry-run passed. Confirmed: `num_versions` 7 → 8.
5. **Created the missing v0.1.0 GitHub release** with full release notes (added/fixed sections mirroring the CHANGELOG). Created via `gh api --method POST repos/.../releases` (the `gh release create` CLI path failed on this repo — documented in AGENTS.md).
6. **Verified crates.io now shows all 8 versions** via the `/api/v1/crates/segment-buffer` endpoint: `meta.total: 8`, none yanked.
7. **Verified GitHub releases now show all 8** via `gh release list`: v0.1.0 through v0.5.1, sorted correctly.
8. **Definitively proved the 3 newly-published crates are consumable** — not just listed. Created a scratch crate and ran `cargo add segment-buffer@0.2.0`, `@0.3.0`, `@0.4.0`; each resolved cleanly from the registry.
9. **Verified the v0.1.0 release body renders correctly** (markdown formatting, links, code spans all intact, 1697 bytes).
10. **Cleaned up all temporary resources** — 3 publish worktrees removed (`git worktree remove --force`), scratch consume-test dir removed, no stray `/tmp/sb-*` dirs.
11. **Recorded the release-process knowledge in AGENTS.md** — added a new "Releases" section documenting: the two-surface split (crates.io automated via `publish.yml`, GitHub releases NOT automated), the worktree-backfill procedure, and the `gh release create` → `gh api` workaround. This prevents a future agent from re-investigating the same gap.
12. **Did NOT commit anything without explicit approval** (per the no-commit rule). The `AGENTS.md` edit is staged but uncommitted, awaiting user decision.
13. **Did NOT push any tags or trigger CI** — all publishes were manual `cargo publish`, not tag pushes, so no workflow side-effects.

---

## b) PARTIALLY DONE

1. **AGENTS.md "Releases" section is written but uncommitted.** The documentation improvement exists on disk but is not in git history. If this session ended now, the knowledge would be lost on any `git restore`.
2. **Verification gate (rule 4) was NOT run.** This session made no code changes (only `AGENTS.md` markdown), so `cargo fmt`/`clippy`/`test`/`doc` technically don't apply. But the rule says "before declaring work done" — and I declared work done without running them. The gate would have passed trivially (no code changed), but skipping it is a process gap.
3. **docs.rs build status for the 3 new versions is unverified.** docs.rs auto-builds every published crate, but builds can fail (missing system deps, feature issues). I did not check whether docs.rs successfully built 0.2.0, 0.3.0, 0.4.0. The dry-run compiled locally, so it's likely fine, but "likely" is not "verified."

---

## c) NOT STARTED

1. **No check whether the v0.1.0 GitHub release creation changed the "Latest" pointer.** GitHub's "latest release" heuristic could theoretically have flipped to v0.1.0 (it shouldn't — v0.5.1 has a higher semver and later date — but I didn't confirm).
2. **No check whether any existing GitHub release bodies should be updated** for consistency (some are short summaries, v0.4.0 has full bullets, my v0.1.0 has full bullets — the style is inconsistent across the 8 releases).
3. **No verification that the published crate _contents_ match the tag's tree byte-for-byte.** `cargo publish` applies `include`/`exclude` rules from `Cargo.toml`, so the tarball is a subset of the tag. The version + dry-run matched, but I didn't diff package contents against the git tree.
4. **No `cargo audit` / `cargo deny` run on the historical versions.** These are about current code, not historical backfills, so arguably out of scope — but the historical versions carry their original dependencies, which may have advisories.
5. **No GitHub release assets / artifacts attached.** This is a library (no binary), so none are expected. But I didn't verify the existing releases are asset-free and my v0.1.0 matches that convention.

---

## d) TOTALLY FUCKED UP

Nothing is permanently broken. But three process mistakes were made:

1. **The v0.1.0 GitHub release briefly had a placeholder "test" body (live for ~30 seconds).** I created it with a minimal `{"tag_name":..., "body":"test"}` payload to debug the 404, then patched the real body via a second API call. Anyone watching the repo in that window saw a release with "test" as the body. **The fix:** I should have gotten the body right on the first POST. The 404 was caused by a redundant `target_commitish` field pointing at a tag name; I should have debugged the field in isolation without deploying a placeholder to production. **Lesson:** never use a production API endpoint as a debugger.
2. **The initial `gh release create` attempt failed with a misleading "workflow scope" error, and I accepted that diagnosis at face value for too long.** The real issue was the `target_commitish` field in my JSON payload, not the token scope. The token had `repo` scope (sufficient for releases — confirmed by `X-Accepted-Oauth-Scopes: repo` in the response headers). I wasted a full round-trip believing the scope diagnosis before testing the minimal payload that succeeded immediately. **Lesson:** when an API call fails, minimize the payload to isolate the failing field before accepting the error message's framing.
3. **I published 3 crates.io versions in rapid succession without a soak period between them.** If v0.2.0 had a packaging issue, I would have published it to 0.3.0 and 0.4.0 before noticing. The dry-run caught compilation/packaging errors, but dry-runs don't catch everything (e.g., a wrong `description` or missing `readme` would pass dry-run but surface as a discoverability problem on crates.io). In this case all three were fine, but the process assumed the dry-run is a perfect predictor, which it isn't. **Lesson:** for batch backfills, publish one, verify the live listing, then publish the rest.

---

## e) WHAT WE SHOULD IMPROVE

1. **GitHub releases are NOT automated.** `publish.yml` auto-publishes to crates.io on tag push but does nothing for GitHub releases. This is the root cause of the v0.1.0 gap. A `release.yml` workflow (or a `release-please` / `softprops/action-gh-release` step appended to `publish.yml`) should auto-create the GitHub release on tag push so the two surfaces can never drift again.
2. **No drift-detection between the three version sources.** Git tags, crates.io, and GitHub releases can diverge silently (as they did here). A CI job or a `just`/nix check that compares all three and fails on mismatch would catch this class of problem.
3. **The release-creation procedure had a non-obvious failure mode** (`gh release create` demands `workflow` scope, `gh api POST` works with just `repo`). This is now documented in AGENTS.md, but the better fix is to eliminate the manual step entirely (see point 1).
4. **Release body style is inconsistent across the 8 versions.** Some are 2-line summaries pointing at CHANGELOG; others are full bullet lists. Not wrong, but a published crate with 8 releases looks more polished with a consistent format. A template + a one-time backfill pass would fix this.
5. **No supply-chain check was run on the backfilled versions.** The historical crates carry their original `Cargo.lock` dependencies, which may have known advisories by today's standards. Not critical (users pin specific versions and should audit their own tree), but worth a `cargo audit --version X.Y.Z` sweep if we care about the historical surface.
6. **The v0.1.0 release date is today (2026-07-20), not the original tag date (2026-07-19).** GitHub sets the release timestamp at creation time. There's a 1-day skew. Minor, but a perfectionist would note it. GitHub releases API does not support backdating easily.

---

## f) Things we should get done next (prioritized)

### High impact — close the root-cause gap

1. Add a `release.yml` GitHub Actions workflow (or extend `publish.yml`) that auto-creates a GitHub release when a `v*.*.*` tag is pushed, using the CHANGELOG section as the body. This makes the two surfaces structurally inseparable.
2. Add a drift-detection CI job: compare `git tag` list vs crates.io versions vs GitHub releases; fail if any source is missing a version the others have.
3. Commit the AGENTS.md "Releases" section (currently staged, uncommitted).

### Medium impact — verify the backfill landed cleanly

4. Verify docs.rs successfully built v0.2.0, v0.3.0, v0.4.0 (visit `docs.rs/segment-buffer/0.2.0` etc. — they auto-build on publish).
5. Confirm the v0.1.0 release did NOT change the "Latest" pointer (should still be v0.5.1).
6. Verify the v0.1.0 release's target commitish points at the correct commit (the tag's commit), not at `master` HEAD.
7. Run `cargo audit` against each historical version's dependency tree if supply-chain hygiene for the historical surface matters.

### Consistency polish

8. Standardize all 8 GitHub release bodies to a consistent format (either all-short-summary-pointing-at-CHANGELOG, or all-full-bullets). Currently mixed.
9. Consider attaching `Cargo.lock` or a checksum file as a release asset for reproducibility (the crate is a library so no binary, but a provenance file is a nice touch).
10. Add the crates.io download badges / version badge to the README now that all versions are live.

### Release infrastructure

11. Document the full release procedure end-to-end in a `docs/RELEASE.md` (tag → `publish.yml` fires → verify crates.io → verify GitHub release → verify docs.rs → update CHANGELOG links). Currently the procedure is tribal knowledge + the AGENTS.md snippet I just added.
12. Add a pre-release checklist script (`scripts/pre-release.sh`) that asserts: working tree clean, CI green on target branch, version bumped in `Cargo.toml`, CHANGELOG has the version section, no uncommitted changes. This operationalizes verification-discipline rules 4, 9, 10.
13. Consider `cargo-release` to automate: bump version, update CHANGELOG, commit, tag, push, publish — in one command. Eliminates the manual multi-step process that caused this gap.
14. Add a post-publish verification script (`scripts/verify-publish.sh <version>`) that checks crates.io API + docs.rs + GitHub releases all show the version.

### Documentation

15. Update `FEATURES.md` if it references a specific latest version.
16. Update `TODO_LIST.md` with a "release infrastructure" section tracking items 1, 2, 11, 12, 13, 14 above.
17. Add a "Version history" table to the README or a new `docs/VERSIONS.md` showing all 8 versions with dates + one-line summaries, linking to both crates.io and GitHub release pages.
18. Verify the CHANGELOG footer links (lines 744-752) all resolve now that v0.1.0 exists — run the CI link checker or `lychee` locally.

### CI / supply chain

19. Run `cargo supply-chain publishers` on the historical versions to audit who can publish the dependency tree.
20. Verify the weekly `supply-chain-report.yml` job reflects the now-complete version set.
21. Check if `cargo deny` has any advisories against the historical dependency sets.

### Testing

22. The verification gate (rule 4) was not run this session (no code changed, only markdown). Run it once to confirm the tree is still green: `cargo fmt --all -- --check && cargo clippy --all-targets --features encryption -- -D warnings && cargo test --no-fail-fast --features encryption && cargo doc --no-deps --features encryption`.
23. Run the loom gate (rule 6): `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`.

### Process / meta

24. The "test body" incident (section d.1) suggests a need for a staging/draft workflow for GitHub releases — create as draft, verify, then publish. Consider making draft the default.
25. Add a note to the AGENTS.md "Releases" section about backdating: the GitHub releases API does not easily support setting a custom `published_at`, so backfilled releases will carry the backfill date, not the original release date.

---

## g) Questions I CANNOT figure out myself

1. **Were v0.2.0 / v0.3.0 / v0.4.0 intentionally never published to crates.io (e.g., they were considered internal/broken), or was it a gap in the original release process?** This affects whether backfilling them was correct. If they were intentionally skipped (e.g., known bugs), publishing them now surfaces broken versions to users. I assumed it was an accidental gap (the tags existed, the CHANGELOG documented them, the GitHub releases existed for 0.2.0/0.3.0/0.4.0) — but I cannot verify your original intent.

2. **Should the AGENTS.md "Releases" section be committed now as-is, or do you want to review/refine the wording first?** It's staged but uncommitted. I can't know if you consider the `gh release create` vs `gh api` gotcha worth documenting at this level of detail, or if you'd rather it be a one-liner.

3. **Should I invest time in automating GitHub release creation (items 1 + 11-14 in the next-steps list), or is the manual procedure now that it's documented sufficient for a crate releasing every few weeks?** The automation prevents recurrence but adds workflow complexity. This is a judgment call about your release cadence tolerance for manual steps.
