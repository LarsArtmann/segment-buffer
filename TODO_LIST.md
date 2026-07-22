# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher, parallel flush
workers) live in [ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (recent entries
stay until the next CHANGELOG cut, then move out).

---

## Documentation polish (README + rustdoc)

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a narrow viewport (mobile-width). The ToC, Status block, Cargo features table, and the new `iter_from` / `open_with_report` code blocks all need a human eye — lychee catches link and anchor drift, not rendering regressions. _Standing item since 15-52 §b.3; Cargo features table and the two new code sections added this session widen the surface that needs verification._ Effort: ~15min. _(User action — requires a browser, not a code change.)_

## CI / gate hardening

- `[ ]` **Set up `master` branch protection with lychee + html-root-url as required checks.** As of this session, `gh api repos/LarsArtmann/segment-buffer/branches/master/protection` returns 404 — the branch is **not protected at all**, so the `link-check`, `html-root-url`, and the new `actionlint` CI jobs can be bypassed by a direct push. The previous TODO assumed protection existed and asked only whether the two jobs were in the required-checks list; the premise was wrong. Decision needed: enable protection (with required checks = `test`, `msrv`, `msrv-consistency`, `html-root-url`, `supply-chain`, `loom`, `link-check`, `actionlint`) and require PR review, or accept the unprotected state. _User decision — requires admin access to the repo settings, not a code change._

## User-decision items (need input, not execution)

- `[ ]` **`update-old-docs` pass on the 14+ historical `2026-07-2*` snapshots.** Out of `docs-health` scope (living docs only). Many snapshots now describe resolved state ("81 unit tests", "CI red for 5 runs", "Cargo.toml still 0.4.2") and would mislead a reader who treats them as current. Decision needed: annotate all, annotate top 3-4 highest-traffic, leave as-is, or delete stale ones. _Deferred across 05-14 §g Q2, 06-27 §f item 6, 16-13 §g Q3._
- `[ ]` **Ship `v0.5.2` doc/CI patch?** The unreleased changes (README polish: features table + `iter_from`/`append_all`/`open_with_report` examples; rustdoc `# Concurrency` section, `doc(alias)`, examples cross-link; `actionlint` gate + CI job; lychee redirect rationale; plus the prior session's deny.toml cleanup, CI html_root_url job, verify-gate.sh comment) are repo-internal, not user-facing — though the README and rustdoc additions ARE user-visible on docs.rs and GitHub. Probably worth a patch release so the docs.rs page stops showing the v0.5.1 surface that omits `iter_from` / `append_all` from the README. The call is the maintainer's. See CHANGELOG `[Unreleased]`.

---

## See also

- [ROADMAP.md](ROADMAP.md) — long-term direction: async I/O, envelope v2
  (streaming CBOR early-stop, Blake3 checksum, compression negotiation,
  metadata block, streaming cipher), second `SegmentStore` impl.
- [CHANGELOG.md](CHANGELOG.md) — shipped work.
- [`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
  — full rationale for the envelope v2 deferrals.
- [`docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md`](docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md)
  — Pareto plan and addendum covering the perf batch that shipped
  (tuning guide, Vec recycling, background-flush pattern example).
