# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher, nightly benchmark
CI) live in [ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md); current capabilities in
[FEATURES.md](FEATURES.md).

Settled design decisions (health-check primitive rejected, panic-free API
shipped, `mtime` gap formally accepted, `segment_count` type documented,
flush-worker rejected) are recorded in [ROADMAP.md](ROADMAP.md) § Non-goals,
[CHANGELOG.md](CHANGELOG.md), and [AGENTS.md](AGENTS.md) — they do not belong
here.

Status legend: `[ ]` pending · `[~]` in progress.

---

## Durability (release-scoped)

- `[ ]` **Flip the default `DurabilityPolicy` from `Segment` to `Throughput`
  with a deprecation note.** The planned one-release backward-compatibility
  window has elapsed (the enum shipped in v0.5.0; v0.5.5 is current). Cloud-sync
  deployments where the cloud holds the durable copy are the target use case, so
  `Throughput` (no fsync) is the correct default; `Maximal` / `Segment` stay
  selectable for standalone-queue deployments. Update the default in
  `SegmentConfig::default()` / builder, add a deprecation callout in the rustdoc
  and `docs/DOMAIN_LANGUAGE.md`, and note the flip in the release CHANGELOG.
  Source: AGENTS.md § Durability model, `docs/status/archived/2026-08-04_04-15_*`.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, Mermaid diagram, and the `iter_from` / `open_with_report` code blocks
  all need a human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15 min. _(User action — requires a
  browser, not a code change.)_

---

## CI / process

All CI / process items from this section were resolved in the post-v0.5.6
CI-hardening session:

- ~~**Audit CI vs local gate parity.~~ Fixed four divergences:
  - Added `clippy(fuzz)` to CI `test` job (was local-only).
  - Added `RUSTDOCFLAGS="-D warnings"` to CI doc build (local was stricter).
  - Added `cargo-lock` (`cargo fetch --locked`) and `msrv-consistency`
    (`check-msrv.sh`) to `verify-gate.sh` (were CI-only).
  - Updated gate count from 15 to 17 in AGENTS.md.
- ~~**Add clippy with full lint stack to the MSRV CI job.~~ Added `components:
  clippy` and two clippy steps (default + encryption) to the `msrv` job.
- ~~**Improve `check-changelog-links.sh` robustness.~~ Added `GITHUB_TOKEN`
  support (60/hr → 5000/hr), HTTP 403 rate-limit detection with graceful
  degradation (exit 0 + warning), and a `check_tag()` helper to DRY the
  curl logic. CI `changelog-links` job now passes `secrets.GITHUB_TOKEN`.
- ~~**Add `--list` and `--only=` options to `verify-gate.sh`.~~ Implemented
  with a `should_run()` gate filter, slug-based matching, unknown-name
  validation, and full `--help` documentation.

---

## See also

- [ROADMAP.md](ROADMAP.md) — long-term direction: async I/O, envelope v2
  (streaming CBOR early-stop, Blake3 checksum, compression negotiation,
  metadata block, cipher auto-detection), streaming cipher, second
  `SegmentStore` impl, nightly benchmark CI workflow, jscpd duplication gate.
- [CHANGELOG.md](CHANGELOG.md) — shipped work.
- [FEATURES.md](FEATURES.md) — current capability inventory by status.
- [`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
  — full rationale for the envelope v2 deferrals.
- [`docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md`](docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md)
  — Pareto plan and addendum covering the perf batch that shipped
  (tuning guide, Vec recycling, background-flush pattern example).
