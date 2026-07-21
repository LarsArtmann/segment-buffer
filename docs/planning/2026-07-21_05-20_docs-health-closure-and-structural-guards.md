# Plan — Docs-health closure + structural guards (2026-07-21)

**Created:** 2026-07-21 05:20 CEST
**Author:** Crush, prompted by Lars
**Status:** **SHIPPED 2026-07-21 ~06:00 CEST** — all 25 executable tasks landed; `scripts/verify-gate.sh --all` reports 13/13 green (including the 3 new steps: lychee, html_root_url, and the existing cargo audit/deny/nix that were skipped in the prior session). 4 user-decision items (U1–U4) remain deferred.
**Predecessor:** `docs/status/2026-07-21_05-14_docs-health-audit-living-doc-drift-fixed-gate-incomplete.md`

**Format note:** The `pareto-planning` skill contract says HTML; this repo has consistently renegotiated to markdown for all 4 prior planning docs and 9 status reports. Following repo convention (markdown). The HTML-vs-MD question is deferred as a user decision (item U4).

---

## Context

The docs-health audit (status report `2026-07-21_05-14`) fixed 1 Critical + 1 Med-High + 9 Medium drift items across 7 living docs, but left four classes of gap:

1. **Verification incomplete** — 5 of 7 gates ran; `nix flake check`, `cargo audit`/`deny`, and lychee were skipped. Two edits are therefore unverified (anchor + rand snippet).
2. **Known doc-quality drift unfixed** — 8 items the audit _noticed_ but didn't address (spotted while reading other docs).
3. **No structural guards** — nothing prevents the same drift from recurring next release.
4. **User decisions pending** — 3 questions in §g of the status report, plus the commit/release decision.

This plan closes all four. Every task ≤12 min. Decisions needing the user are clearly marked and deferred.

### Correction to the status report

§d.1 of the status report flagged my AGENTS.md anchor fix (`#durability-model-shipped-in-v050`) as "may be broken." **That was a false alarm.** GitHub's slugify (verified via the `github-slugger` algorithm: lowercase → strip non-`\w\- ` chars → spaces to hyphens) does strip dots, so the anchor is correct. lychee would still confirm definitively; the slug computation is settled.

### What this plan does NOT include (correctly out of scope)

- **`update-old-docs` pass on historical snapshots.** Different skill; the docs-health boundary is explicit. Tracked as U2.
- **v0.5.2 release cut.** Needs user approval (Q1). Tracked as U1.
- **Commit + push.** Needs user approval. Tracked as U5.

---

## Pareto breakdown

### Tier 1 — The 1% that delivers 51%: close the verification gaps

Without this, every other claim in this session is unverified. The whole point of the docs-health audit was to ship honest docs; shipping unverified fixes is the opposite.

### Tier 2 — The 4% that delivers 64%: the 8 noticed-but-unfixed drift items

Real factual drift in living docs. Each is a 5–10 min fix once verified against code. Cumulatively they bring the doc set from "headline drift fixed" to "actually current."

### Tier 3 — The 20% that delivers 80%: structural guards

The recurring failure mode across the 7 prior 2026-07-20 reports is "drift was caught late because no gate caught it early." Landing the guards (`lychee` in `verify-gate.sh`, `missing_panics_doc` lint, `html_root_url` sync check) makes the next docs-health pass shorter.

### Tier 4 — The remaining 20%: deferrals and decisions

Items that need the user (release scope, HTML-vs-MD, commit approval, historical annotation scope).

---

## Comprehensive plan — atomic tasks ≤12 min each

Sort order: Pareto tier → impact × value ÷ effort within tier. Dependencies in last column.

### Tier 1 — Verification (close the §d gaps first)

| ID   | Task                                                                                                                            | Effort | Impact   | Customer value                             | Depends on |
| ---- | ------------------------------------------------------------------------------------------------------------------------------- | ------ | -------- | ------------------------------------------ | ---------- |
| T1.1 | Run `nix flake check` and capture exit code                                                                                     | 5 min  | Critical | Closes §b.1 of status report               | —          |
| T1.2 | Run `cargo audit` + `cargo deny check` via `nix run nixpkgs#cargo-audit` and `nix run nixpkgs#cargo-deny`                       | 10 min | Critical | Closes §b.2; honors AGENTS rule 5          | —          |
| T1.3 | Run lychee locally: `nix run nixpkgs#lychee -- --config .github/lychee.toml '*.md' 'docs/**/*.md' 'fuzz/README.md'`             | 8 min  | Critical | Closes §b.3; verifies AGENTS anchor fix    | —          |
| T1.4 | Verify the CIPHERS.md rand 0.10 snippet compiles via a scratch crate (or extract to doctest as T3.3 and let `cargo test` check) | 12 min | High     | Closes §d.2 of status report               | —          |
| T1.5 | Re-read the tails (lines 200+) of the 8 truncated 2026-07-2* files for any missed drift                                         | 10 min | Medium   | Closes §d.3; honors "READ ALL" instruction | —          |
| T1.6 | If T1.4 surfaces an API error, fix the CIPHERS snippet (otherwise no-op)                                                        | 5 min  | High     | Honest doc                                 | T1.4       |

### Tier 2 — Doc-quality drift fixes (the noticed-but-unfixed items)

| ID   | Task                                                                                                                                                 | Effort | Impact  | Customer value                      | Depends on |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------- | ----------------------------------- | ---------- |
| T2.1 | Fix AGENTS.md "Code conventions": describe `test_config` accurately as `FlushPolicy::Batch(4)`; note concurrency tests use `Manual` (rule 7)         | 5 min  | Medium  | Honest repo context for AI sessions | —          |
| T2.2 | Verify AGENTS.md "Architecture & data flow" diagram still matches the store-trait dispatch (post-v0.5.0); fix labels if stale                        | 10 min | Low-Med | Architecture doc current            | —          |
| T2.3 | Collapse the duplicate "Comparison tables rot" disclaimers in README.md (lines 163 + 166) to one                                                     | 3 min  | Low     | Polish                              | —          |
| T2.4 | Update docs/RELEASE.md step 1 example from `version = "0.4.1"` / `--precise 0.4.1` to version-neutral (`<old>` → `<new>`)                            | 5 min  | Low-Med | Runbook doesn't mislead             | —          |
| T2.5 | Rewrite docs/PERFORMANCE.md "What the envelope costs" paragraph: the "30–65% slower vs v0.1.0" headline is obsolete (2.3× faster since CCtx pooling) | 8 min  | Medium  | Perf doc matches README claim       | —          |
| T2.6 | Spot-check FEATURES.md numeric claims against source: 597M events (provenance?), 187811 fuzz runs, ~12ns stats, ~21× for_each_from                   | 10 min | Medium  | Trust in FEATURES                   | —          |
| T2.7 | Read docs/CROC_LESSONS.md; confirm still wanted + accurate (or mark as historical if stale)                                                          | 5 min  | Low     | No orphan docs                      | —          |
| T2.8 | Check renovate.json vs .github/dependabot.yml for duplication or stale ignores                                                                       | 5 min  | Low     | No split-brain dep config           | —          |
| T2.9 | Verify AGENTS.md "top-level doc example is `#![no_run]`-gated" claim is still true (the `include_str!` removal may have changed this)                | 3 min  | Low     | Honest conventions doc              | —          |

### Tier 3 — Structural guards (prevent recurrence)

| ID   | Task                                                                                                                                                                                          | Effort | Impact | Customer value                            | Depends on |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ------ | ----------------------------------------- | ---------- |
| T3.1 | Add a lychee step to `scripts/verify-gate.sh` mirroring the CI config (the recurring TODO across 4 prior reports)                                                                             | 8 min  | High   | Next docs-health pass doesn't repeat §d.1 | T1.3       |
| T3.2 | Add a "docs-health cadence" note to AGENTS.md (living docs re-audited on release, not just when remembered)                                                                                   | 5 min  | Medium | Process enforcement                       | —          |
| T3.3 | Convert docs/CIPHERS.md bring-your-own snippet to a cfg-gated doctest (copy the README encryption-example pattern); pull `chacha20poly1305` + `rand` as encryption-feature dev-deps if needed | 12 min | High   | Future API drift caught by `cargo test`   | T1.4       |
| T3.4 | Enable `clippy::missing_panics_doc` + `clippy::missing_errors_doc` in `src/lib.rs`; fix any violations surfaced (standing TODO from 08-42 §e.1)                                               | 12 min | Medium | Prevents doc-section regression           | —          |
| T3.5 | Add `scripts/check-html-root-url.sh` asserting `html_root_url` in src/lib.rs matches `version` in Cargo.toml; wire into `verify-gate.sh`                                                      | 10 min | Medium | Kills the recurring html_root_url rot     | —          |
| T3.6 | Add a "Skill gate completeness" note to `~/.config/crush/skills/docs-health/SKILL.md` VERIFY step 7: list `nix flake check` + lychee alongside cargo                                          | 8 min  | Medium | Next agent doesn't repeat my §b gaps      | —          |

### Tier 4 — Deferrals (need user decision)

| ID  | Task                                                                                                          | Why deferred                                                                                             |
| --- | ------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| U1  | Ship the 7-file doc fix as v0.5.2 patch (CHANGELOG entry + html_root_url bump + tag) or hold for next feature | Release-cadence decision; cannot decide without user. AGENTS rule: no release without explicit approval. |
| U2  | Run `update-old-docs` skill on the 14 historical 2026-07-2* snapshots                                         | Different skill's scope; docs-health boundary is explicit.                                               |
| U3  | Commit the doc fixes (and any Tier 2/3 additions)                                                             | No-commit-without-explicit-approval rule.                                                                |
| U4  | Decide HTML-vs-Markdown for status/planning reports (9th+5th consecutive markdown deviation from skill)       | Skill-contract renegotiation; user call.                                                                 |

---

## Execution order

T1.1 → T1.2 → T1.3 → T1.4 → T1.5 → T1.6 (if needed) → T2.1–T2.9 (parallel where independent) → T3.1–T3.6 → report → defer U1–U4.

**Stop conditions:**

- Any Tier 1 gate red → stop, report, do not proceed to Tier 2.
- Any Tier 2 fix surfaces a code bug (not a doc bug) → stop, report, do not fix code without scope check.
- User interruption → respect it.

**Non-goals during execution:**

- No commits without explicit approval (U3).
- No release tags (U1).
- No historical-doc annotation (U2).
- No skill-contract renegotiation (U4).

---

## Verification at plan completion

Before declaring the plan done:

- [ ] All Tier 1 tasks green with exit codes captured.
- [ ] All Tier 2 fixes applied and re-verified against code.
- [ ] All Tier 3 guards landed and run green at least once.
- [ ] `git status` shows every modified file explained.
- [ ] `gh run list --limit 4` still green on master (or unchanged, since nothing is pushed).
- [ ] Report-back table delivered to chat with per-task status.
