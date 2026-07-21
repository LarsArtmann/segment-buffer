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

- `[ ]` **Cargo features table in README.** Users see `--features encryption` in Install but no enumeration of `default = []`, `encryption`, `loom` (test-only), `fuzz` (test-only, not semver). Add a 4-row table near the install block. _Standing item since 15-20 §c.4; reiterated 15-52 §e.5._ Effort: ~20min.
- `[ ]` **`iter_from` example in README drain loop.** The README shows `read_from` but not the owned-item iterator added in v0.5.0. One fenced block alongside the existing drain loop. _Standing item since 15-20 §c.2._ Effort: ~15min.
- `[ ]` **`append_all` one-liner in README.** The batch primitive (v0.4.1) is documented in FEATURES.md and `docs/PERFORMANCE.md` §Tuning but not in README. _Standing item since 15-20 §c.2._ Effort: ~10min.
- `[ ]` **`open_with_report` crash-recovery example in README.** The crate's defining feature has prose but no code in the README. The example binary `examples/crash_recovery.rs` exists; reference or excerpt it. _Standing item since 15-20 §c.5._ Effort: ~30min.
- `[ ]` **Resolve 2 lychee-flagged redirect URLs in README.** Lychee reports 2 redirects (consider replacing with the resolved URL). Run `nix run nixpkgs#lychee -- --config .github/lychee.toml README.md` to enumerate. _Standing item since 15-52 §c._ Effort: ~10min.
- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a narrow viewport (mobile-width). The ToC and Status block were restructured in 15-52; lychee catches links, not rendering. _Standing item since 15-52 §b.3._ Effort: ~15min.

## Rustdoc discoverability

- `[ ]` **`doc(alias = "queue" | "spool" | "wal")` on `SegmentBuffer`.** Improves rustdoc search discoverability for users coming from other ecosystems. One-line attribute in `src/lib.rs`. _Standing item since 08-42 §f.8._ Effort: ~5min.
- `[ ]` **`# Concurrency` section on `SegmentBuffer`.** Document MPMC semantics (parking_lot::Mutex, mutex-never-held-across-I/O, MPMC stress test reference) in the rustdoc, not just AGENTS.md. _Standing item since 08-42 §f.11._ Effort: ~20min.
- `[ ]` **Cross-link `examples/` from crate-root rustdoc** (`src/lib.rs` `//!`). Currently the examples directory is invisible from `cargo doc` output. _Standing item since 08-42 §f.17._ Effort: ~15min.

## CI / gate hardening

- `[ ]` **Add `actionlint` to `scripts/verify-gate.sh`.** Standing item since 06-27 §f item 32. YAML parse is the floor; actionlint catches expression syntax (`${{ }}`), `needs:` cycles, and outdated action versions. Wire next to the existing lychee step. Effort: ~20min.
- `[ ]` **Verify `lychee` and `html-root-url` are in the branch-protection required-checks list.** Both now run as CI jobs (`link-check` since v0.4.1; `html-root-url` since this session) but may not be required — `gh api repos/LarsArtmann/segment-buffer/branches/master/protection`. _Standing item since 06-27 §f item 8 (lychee) and new this session (html-root-url)._ Effort: ~10min.

## User-decision items (need input, not execution)

- `[ ]` **`update-old-docs` pass on the 14+ historical `2026-07-2*` snapshots.** Out of `docs-health` scope (living docs only). Many snapshots now describe resolved state ("81 unit tests", "CI red for 5 runs", "Cargo.toml still 0.4.2") and would mislead a reader who treats them as current. Decision needed: annotate all, annotate top 3-4 highest-traffic, leave as-is, or delete stale ones. _Deferred across 05-14 §g Q2, 06-27 §f item 6, 16-13 §g Q3._
- `[ ]` **Ship `v0.5.2` doc-only patch?** The unreleased changes (AGENTS test count 81→82, AGENTS examples list +2, FEATURES test count 81→82, plus this session's deny.toml cleanup, CI html_root_url job, verify-gate.sh comment, TODO_LIST rebuild) are repo-internal, not user-facing. Almost certainly not worth a patch release — but the call is the maintainer's. See CHANGELOG `[Unreleased]`.

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
