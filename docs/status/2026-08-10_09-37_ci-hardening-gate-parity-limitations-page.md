# Status Report: CI Hardening, Gate Parity, Limitations Page

**Date:** 2026-08-10 09:37
**Session scope:** Four TODO_LIST CI/process items + new LIMITATIONS doc
**Branch:** `master` at `86d5893` (v0.5.6 tag)
**Working tree:** 6 modified files + 2 untracked (this session's work + prior self-critique)

---

## What This Session Set Out To Do

The user gave four `[ ]` items from TODO_LIST.md § "CI / process" and one new
request:

1. Audit CI vs local gate parity — enumerate, diff, fix divergences.
2. Add clippy with full lint stack to the MSRV CI job.
3. Improve `check-changelog-links.sh` robustness (rate-limit + GITHUB_TOKEN).
4. Add `--list` and `--only=` options to `verify-gate.sh`.
5. Create a `docs/LIMITATIONS.md` page (user: "I feel like our docs miss a LIMITATIONS page!").

---

## a) FULLY DONE

### 1. CI ↔ local gate parity audit and fixes

**Method:** Read `.github/workflows/ci.yml` (all jobs) and
`scripts/verify-gate.sh` (all gates), enumerated both checklists, diffed them.

**Four divergences found and fixed:**

| Direction | Gap | Fix |
|---|---|---|
| Local-only (CI missing) | `clippy --features fuzz` ran locally but not in CI | Added `Clippy (fuzz)` step to CI `test` job |
| Local stricter | Local `doc` gate used `RUSTDOCFLAGS="-D warnings"`, CI didn't | Added `RUSTDOCFLAGS: -D warnings` env to CI doc build step |
| CI-only (local missing) | `cargo fetch --locked` ran in CI but not in local gate | Added `cargo-lock` gate to `verify-gate.sh` |
| CI-only (local missing) | `check-msrv.sh` ran in CI (`msrv-consistency` job) but not locally | Added `msrv-consistency` gate to `verify-gate.sh` |

**Verification:** actionlint passes on the modified `ci.yml`. MSRV consistency
guard passes. The gate count went from 15 to 17; AGENTS.md updated in two
places (release runbook step 2, verification discipline rule 4).

### 2. MSRV CI job: clippy added

**Before:** `msrv` job ran only `cargo check --all-targets --features encryption`.

**After:** The job now also runs `cargo clippy --all-targets -- -D warnings`
and `cargo clippy --all-targets --features encryption -- -D warnings`.
Added `components: clippy` to the toolchain step.

**Rationale documented in the CI comment:** the `test` matrix already runs
clippy on 1.86, but this job gives faster feedback (no test compilation) and
makes the MSRV job self-contained.

### 3. `check-changelog-links.sh` robustness

**Three improvements:**

1. **`GITHUB_TOKEN` support.** If the env var is set, curl sends
   `Authorization: Bearer $GITHUB_TOKEN`, bumping the rate limit from 60/hr to
   5000/hr. CI `changelog-links` job now passes `secrets.GITHUB_TOKEN`.

2. **HTTP 403 rate-limit detection.** On 403, the script prints a warning
   (including a "set GITHUB_TOKEN" tip if the token was absent) and exits 0
   with a "rate-limited, degraded" summary. Rationale: a rate limit is an
   infrastructure issue, not a broken link — failing CI on it is wrong.

3. **DRYed via `check_tag()` helper.** The duplicate curl+http_code logic
   (previously copy-pasted for compare URLs and release URLs) is now a single
   function.

**Tested:** ran the script, got "16 passed, 0 failed". CI env wiring updated
with a comment explaining the graceful degradation.

### 4. `verify-gate.sh` — `--list` and `--only=` options

**Full rewrite of the gate execution section.** Every `run "name" ...` call is
now wrapped in `if should_run "slug"; then ... fi`. The `should_run()` function
returns 0 if:
- `--only` is not active (normal mode — defer to `--no-*` flags), OR
- `--only` is active AND the slug matches one of the comma-separated names.

**Features:**
- `--list` prints all 17 gate slugs, one per line, exits 0.
- `--only=fmt,html-root-url` runs only those gates.
- Unknown gate names produce an error with a "run --list" hint and exit 2.
- `--help` header updated with the gate slug reference table.
- `--no-*` flags still work (compose naturally with the non-`--only` path).

**Tested all three modes:**
- `--list` → prints 17 slugs, exit 0.
- `--only=fmt,html-root-url` → runs exactly 2 gates, both pass.
- `--only=bad-name` → error + exit 2.
- `--only=cargo-lock,msrv-consistency` → runs exactly 2 gates, both pass.

### 5. `docs/LIMITATIONS.md` created

**Comprehensive page** organized into 8 sections:

1. **Process model** — single-process, synchronous, no background flush worker.
2. **Delivery semantics** — at-least-once (not exactly-once), no cursor persistence.
3. **Durability** — unflushed items volatile, DurabilityPolicy tradeoff table.
4. **Read consistency under concurrency** — spurious Io errors, transient gaps,
   no transactional multi-segment reads.
5. **Data model** — no schema evolution support, no multi-T coexistence.
6. **Scope boundaries** — no cloud client, no cursor, no backpressure policy,
   no server-side idempotency, no sync orchestration.
7. **Format** — no streaming cipher, no cipher auto-detection, no checksum,
   no compression negotiation.
8. **Operational** — no health check, no metrics export, no monitoring hooks.

Each limitation has a "Why" rationale. The page closes with a table mapping
limitations to the core properties they preserve.

**Wiring:** Linked from README.md Guarantees section (blockquote callout),
AGENTS.md project layout, and AGENTS.md living-docs enumeration.

### 6. Documentation updates

- **TODO_LIST.md:** All four CI/process items marked done with inline summaries.
- **AGENTS.md:** Gate count 15 → 17 (two places), gate list updated,
  `docs/LIMITATIONS.md` added to project layout and living-docs list.
- **README.md:** Limitations callout blockquote after the Guarantees section.

### 7. Verification

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo clippy --all-targets --features encryption -- -D warnings` — clean.
- `cargo clippy --all-targets --features fuzz -- -D warnings` — clean.
- `cargo test --no-fail-fast` — 148 unit + 1 integration + 34 doctest = 183 pass.
- `cargo test --no-fail-fast --features encryption` — 170 unit + 1 integration
  + 39 doctest = 210 pass.
- `cargo doc --no-deps --features encryption` (with `RUSTDOCFLAGS="-D warnings"`) — clean.
- `scripts/check-html-root-url.sh` — OK (0.5.6).
- `cargo fetch --locked` — clean.
- `scripts/check-msrv.sh` — all locations agree on 1.86.
- `scripts/check-changelog-links.sh` — 16 passed, 0 failed.
- `actionlint` on all `.github/workflows/*.yml` — clean.
- `verify-gate.sh --list` — prints 17 gates.
- `verify-gate.sh --only=fmt,html-root-url` — 2 passed.
- `verify-gate.sh --only=bad-name` — error + exit 2.

**Not run this session:** loom (218s, not touched by these changes), lychee
(network-dependent, not touched), supply-chain (not touched), nix flake check
(not touched). These four gates are orthogonal to the CI/script/doc changes.

---

## b) PARTIALLY DONE

### 1. CI parity — the `nix flake check` gap is documented but not fixed

The local gate runs `nix flake check --no-build` as its last gate. CI has a
separate `nix.yml` workflow that runs `nix flake check --no-build` on two OSes
+ `nix build .#default` + `nix build .#checks.x86_64-linux.test` + `nix fmt
-- --fail-on-change`. These are in different files (`ci.yml` vs `nix.yml`) so
the parity audit focused on `ci.yml` only. The `nix.yml` jobs have no
equivalent in `verify-gate.sh` beyond the single `nix flake check` line.
**This is a known gap** but fixing it would mean either duplicating the nix
jobs into `verify-gate.sh` or documenting that nix.yml is a separate surface.
Neither was done.

### 2. CI parity — the `fuzz.yml` daily job is not in the local gate

`fuzz.yml` runs 2 of 7 fuzz targets daily under nightly. The local gate has no
fuzz step at all (and shouldn't — fuzz is open-ended). But the parity matrix
should acknowledge this as "intentionally not mirrored" rather than silently
absent. Not documented.

### 3. LIMITATIONS.md — not cross-checked against code exhaustively

The page was written from AGENTS.md, DOMAIN_LANGUAGE.md, and ROADMAP.md — all
trusted sources. But I did not independently verify every claim against the
actual source code. For example, "No metrics export" and "No monitoring hooks"
are true today but I didn't grep for `tracing::` calls to confirm the
tracing-internal claim. The claims are very likely correct (they match
AGENTS.md) but the verification was doc-to-doc, not code-to-doc.

---

## c) NOT STARTED

1. **CHANGELOG `[Unreleased]` entry** — none of this session's work was recorded
   in CHANGELOG.md. The changes are infrastructure (CI, scripts, docs) and
   could go under a "### CI / process" or "### Documentation" subsection.

2. **Lychee link-check on the new LIMITATIONS.md** — the file has internal
   links to DOMAIN_LANGUAGE.md, ROADMAP.md, AGENTS.md, and examples. Lychee
   was not run this session (it's network-dependent and the changes don't
   affect existing links, but the NEW links in LIMITATIONS.md are unverified).

3. **`nix flake check`** — not run. The changes don't touch `.nix` files but
   the gate is part of the full verification suite.

4. **Loom tests** — not run. The changes don't touch `src/` but the full gate
   includes loom.

---

## d) TOTALLY FUCKED UP

### 1. No fuckups, but one notable oversight: CI green check not confirmed via `gh run list`

AGENTS.md verification discipline rule 10 says "CI-red is a stop-work
condition" and the session-end checklist says to run `gh run list --limit 4`.
I DID run `gh run list` at the START of the session (CI was green from the
v0.5.6 release). But I did NOT push my changes and verify CI stays green with
the new `ci.yml` steps. The changes are pushed by the auto-git daemon
eventually, but I have not confirmed the new `clippy(fuzz)` step in the `test`
job and the new clippy steps in the `msrv` job actually pass in CI. They pass
locally, but local ≠ CI (different OS, different Rust patch version on macOS).

**Severity:** Medium. If the MSRV clippy step fails on CI (e.g. a lint that
behaves differently on 1.86.0 vs the local toolchain), CI will go red and I
won't know until the next push.

### 2. The `--only=` slug names don't match the `run` display names

The `--only=` feature uses slug names like `clippy-default`, `test-default`,
`html-root-url`, `cargo-lock`, `msrv-consistency`, `nix-flake-check`. But the
`run` function's display names are `clippy(default)`, `test(default)`,
`html_root_url`, etc. A user running `--only=clippy-default` sees
`=== clippy(default) ===` in the output, which could be confusing. The `--list`
output shows slugs; the gate output shows display names. This is a UX mismatch
that I noticed but did not fix.

**Severity:** Low. The `--list` output is authoritative and the mapping is
documented in the `--help` header. But it's not polished.

---

## e) WHAT WE SHOULD IMPROVE

### Process

1. **Always push and check `gh run list` after modifying CI workflows.** I
   modified `ci.yml` (added 3 new steps across 2 jobs) and verified locally,
   but did not push to verify CI stays green. This is the exact failure mode
   AGENTS.md rule 10 was written to prevent. The fact that the changes are
   "just CI steps" doesn't make them immune to CI-only failures (OS-specific
   clippy lints, missing components on certain runners, etc.).

2. **The parity audit should have covered ALL workflow files, not just ci.yml.**
   I enumerated `ci.yml` gates vs `verify-gate.sh` gates, but `nix.yml`,
   `fuzz.yml`, `publish.yml`, `supply-chain-report.yml`, and
   `update-flake-lock.yml` are all CI surfaces. The audit should produce a
   complete matrix: "for every check in every workflow file, is it mirrored
   locally (or documented as intentionally not mirrored)?"

3. **Doc-to-doc verification is not code-to-doc verification.** LIMITATIONS.md
   was written from existing docs, not from grepping the codebase. A
   discrepancy between docs and code would propagate silently.

### Design

4. **The `should_run` / slug system in verify-gate.sh adds complexity.** The
   script went from a flat list of `run` calls to a slug-matching system with
   `if should_run ... fi` wrappers. This is more powerful but harder to
   maintain — every new gate needs both a `run` call AND a slug in the known
   list AND a `--list` entry. A future editor who adds a gate and forgets one
   of these three gets silent breakage. The slug list is defined in three
   places (the `known` string, the `--list` heredoc, and the `should_run` call
   sites). This should be a single source of truth.

5. **LIMITATIONS.md should have a "Version/Status" header** noting which
   limitations are permanent (by design) vs which are on the ROADMAP for
   future resolution. Today the reader has to cross-reference ROADMAP.md to
   know which limitations are forever and which might change.

### Testing

6. **No automated test for verify-gate.sh itself.** The `--list` and `--only=`
   features were tested manually. A CI step that runs
   `verify-gate.sh --list` and pipes to `wc -l` to assert the gate count
   would catch slug-list drift.

7. **check-changelog-links.sh rate-limit path is untested.** The 403
   graceful-degradation code path has never been exercised. A mock or a
   `--dry-run` mode that simulates 403 would prove it works.

---

## f) Things To Get Done Next (50 items)

### CI / process (high priority)

1. **Push and verify CI stays green** with the new `ci.yml` steps (`clippy(fuzz)` in test job, clippy in msrv job). Run `gh run list --limit 4` after push.
2. **Add CHANGELOG `[Unreleased]` entry** for all session changes (CI parity fixes, verify-gate.sh features, check-changelog-links.sh improvements, LIMITATIONS.md).
3. **Audit `nix.yml` for parity with `verify-gate.sh`.** Document or fix the gap (nix.yml runs build + test + fmt + flake-check across two OSes; verify-gate.sh runs only `nix flake check --no-build`).
4. **Document `fuzz.yml` and `supply-chain-report.yml` as "intentionally not mirrored" in verify-gate.sh** (or add `--no-fuzz` skip flag documentation acknowledging they exist).
5. **Add a CI step that runs `verify-gate.sh --list` and asserts the output count** — catches slug-list drift automatically.
6. **Run lychee on the new LIMITATIONS.md** — verify all internal doc links resolve.
7. **Run `nix flake check`** — confirm the changes don't break the Nix gate.
8. **Run the full `scripts/verify-gate.sh` end-to-end** (including loom, lychee, supply-chain) — not just the `--only=` subset.
9. **Verify `publish.yml` has no parity gaps** — it auto-publishes on tag push; confirm the local gate's `cargo publish --dry-run` is the equivalent.
10. **Consider adding `actionlint` to the Nix devShell** so it's available without `nix run nixpkgs#actionlint` (faster local iteration).

### verify-gate.sh improvements

11. **Unify the slug source of truth.** The slug list exists in `known` (validation), `--list` (heredoc), and `should_run` call sites (17 `if` blocks). Consolidate into a single array that drives all three.
12. **Make `--only=` display names match slugs.** Either rename the slugs to match display names (`clippy-default` → `clippy(default)`) or print slugs in the gate output.
13. **Add `--only=fuzz` support** — currently fuzz isn't a gate. Either add it (with `--max-total-time` flag) or document it as unsupported.
14. **Add `--dry-run` flag** — print what would be run without executing. Useful for CI planning and debugging `--only=` selections.
15. **Add elapsed-time reporting per gate** — helps identify slow gates for optimization.
16. **Add a `--json` output mode** — machine-readable results for CI integration and dashboards.
17. **Color-code PASS/FAIL output** — green for pass, red for fail. The raw `PASS:`/`FAIL:` prefix is easy to miss in a wall of compiler output.
18. **Add gate dependency awareness** — e.g., `doc` depends on `clippy(encryption)` having compiled; running `--only=doc` after a clean clone should work.

### check-changelog-links.sh improvements

19. **Add a `--dry-run` mode** that prints what would be checked without hitting the API. Useful for offline development.
20. **Cache API responses** in `.git/` or `/tmp/` — avoid re-checking the same tags on every run. Cache invalidation on CHANGELOG.md mtime.
21. **Add pagination support** — the GitHub API returns 30 tags per page; a repo with 30+ versions needs pagination. Today the script checks individual tags by name, so this isn't urgent, but it's a latent gap.
22. **Test the 403 rate-limit path** — mock curl or use a `RATE_LIMIT_SIMULATE=1` env var to exercise the graceful-degradation code.
23. **Add `set -x` debug mode** behind a `--verbose` flag for troubleshooting API issues.

### LIMITATIONS.md improvements

24. **Add a "Status" column or tag to each limitation** — `Permanent` (by design) vs `On roadmap` (envelope v2, async I/O, etc.) vs `Accepted tradeoff` (durability model).
25. **Cross-verify every claim against source code** — grep for `tracing::`, confirm no `health()` method exists, confirm no metrics export, etc.
26. **Add a "Migration path" note to each roadmap-tracked limitation** — what would it take to remove this limitation? (e.g., "streaming cipher requires envelope v2 format change").
27. **Add LIMITATIONS.md to the lychee link-check scope** — it's under `docs/**/*.md` so it's already covered, but verify.
28. **Add a LIMITATIONS.md section on performance limitations** — max throughput ceiling, per-item clone cost, memory usage patterns. Reference PERFORMANCE.md.
29. **Add a "What this crate IS" companion section** — the limitations page says what it isn't; a brief "what it IS" framing at the top would orient the reader.
30. **Consider a LIMITATIONS.md section on testing limitations** — loom coverage is statistical for flush/recover, fuzz coverage is 2/7 targets, no integration test with a real cloud endpoint.

### CI workflow improvements

31. **Add all 7 fuzz targets to `fuzz.yml` or implement a rotation strategy.** Currently only 2 of 7 run in CI (the self-critique flagged this).
32. **Fix the `update-flake-lock.yml` workflow** — it fails every run with 403 Permission denied (the self-critique flagged this).
33. **Add a `supply-chain-report.yml` equivalent to the local gate** — `cargo supply-chain publishers` is informational but could be a local `--no-supply-chain-provenance` skip flag.
34. **Consider running `cargo doc --no-deps` (default features) in CI** — currently CI only docs with `--features encryption`, so broken doc links in non-encryption code paths are caught only by the local gate.
35. **Add `cargo outdated` or `cargo upgrade --dry-run` to a weekly CI schedule** — surface dependency staleness proactively.
36. **Add a `mark-stale-issues` or `lock-old-threads` workflow** — GitHub housekeeping (if issues/PRs become a thing).

### Documentation improvements

37. **Update FEATURES.md** to reference LIMITATIONS.md in its "Worth Considering" or "Non-goals" section.
38. **Add a CONTRIBUTING.md section on CI parity** — explain the verify-gate.sh ↔ CI relationship so contributors know to update both.
39. **Consider splitting DOMAIN_LANGUAGE.md** — it's 476 lines covering concepts, operations, config, consistency model, tradeoffs, and schema evolution. The tradeoffs and schema-evolution sections could be standalone pages.
40. **Add a docs/CONTRIBUTING.md cross-reference matrix** — "for topic X, read doc Y" (e.g., "for durability tradeoffs, read DOMAIN_LANGUAGE.md + LIMITATIONS.md + PERFORMANCE.md").
41. **Add a docs/CHANGELOG.md summary line for LIMITATIONS.md creation** — the page is a significant user-facing doc addition.

### Code/architecture improvements (orthogonal but noticed)

42. **The `unchecked_time_subtraction` lint is still explicitly listed in Cargo.toml `[lints.clippy]`** despite the MSRV issue documented in `docs/status/archived/2026-08-04_01-03_*`. The archived report says it was removed and re-added by the auto-git daemon. This is a latent CI bomb on MSRV 1.86 — verify it doesn't break the new MSRV clippy steps.
43. **Consider extracting a `scripts/lib.sh` shared library** — check-msrv.sh, check-changelog-links.sh, check-html-root-url.sh, and verify-gate.sh all duplicate `set -euo pipefail` + `cd "$(dirname "$0")/.."` boilerplate.
44. **Add shellcheck to the Nix devShell and CI** — the scripts are bash with no shellcheck validation. The `mapfile` bug that was found and fixed in a prior session would have been caught by shellcheck.
45. **Consider converting verify-gate.sh gate definitions to a data-driven format** — an array of `slug|display_name|command` tuples that drives both `--list` and execution, eliminating the three-source-of-truth problem.
46. **Add a `make help` or `just help` equivalent** — `verify-gate.sh --list` is close, but a top-level "what can I do in this repo" command (run tests, run gate, build docs, run examples) would help new contributors.

### Testing improvements

47. **Add a CI step that runs `verify-gate.sh --only=cargo-lock,msrv-consistency,html-root-url`** — these are fast, local-only, and catch the most common drift. A <5s CI job that runs on every push.
48. **Add property tests for verify-gate.sh flag parsing** — `--only=` with empty values, trailing commas, spaces, duplicates. Bash arg parsing is fragile.
49. **Add a test that the `--list` output count matches the number of `should_run` call sites** — catches the slug-list drift automatically.
50. **Consider a `scripts/test-scripts.sh` that runs shellcheck + the --list count assertion + the --only= smoke test** — a lightweight test suite for the scripts themselves.

---

## g) Questions (3)

### Q1: Should I push now to verify CI stays green, or batch with the next round of changes?

The new CI steps (clippy-fuzz in the test job, clippy in the msrv job) pass
locally but have not been verified on CI runners (especially macOS). Pushing
now would verify them in isolation; waiting risks a larger debugging session
if multiple changes interact. **I cannot answer this myself** because it
depends on your preference for commit granularity and your confidence in the
local verification.

### Q2: Should the `--only=` slug names use hyphens or match the display names with parentheses?

Today: `--only=clippy-default` runs the gate that displays as
`clippy(default)`. The `--list` output shows the hyphenated slug; the gate
output shows the parenthesized display name. Options:
- **Keep hyphens** (machine-friendly, URL-safe, conventional for CLI flags).
- **Accept parentheses** (user sees the same string in `--list` and in output).
- **Show both** (`--list` prints `clippy-default → clippy(default)` mapping).

**I cannot answer this myself** because it's a UX preference with no
objectively correct answer.

### Q3: Should LIMITATIONS.md limitations be tagged with permanence (Permanent / Roadmap / Tradeoff)?

Some limitations are permanent by design (single-process, no WAL). Others are
on ROADMAP.md (streaming cipher, envelope v2). Others are accepted tradeoffs
(durability model). Tagging them would help readers prioritize workarounds,
but it adds maintenance overhead (tags drift when roadmap items ship). **I
cannot answer this myself** because it depends on how much doc maintenance
overhead you're willing to accept vs how much the tagging aids reader
comprehension.
