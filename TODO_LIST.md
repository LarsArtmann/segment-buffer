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
flush-worker rejected, DurabilityPolicy default flipped to `Throughput` in
v0.6.0, compression-level default changed to 1 in v0.5.7) are recorded in
[ROADMAP.md](ROADMAP.md) § Non-goals, [CHANGELOG.md](CHANGELOG.md), and
[AGENTS.md](AGENTS.md) — they do not belong here.

Status legend: `[ ]` pending · `[~]` in progress.

---

## Testing & code quality

- `[x]` **`Hash` for `FlushPolicy` + `DurabilityPolicy`.** Both are `Copy`
  enums with no interior data. Adding `#[derive(Hash, Eq)]` enables them as
  `HashMap` keys for callers that bucket by policy. Low-effort derive, no
  behavior change.
  Source: Pareto plan D9.

---

## Benchmarks & performance

- `[x]` **Nightly benchmark CI workflow.** A scheduled GitHub Actions job
  (`.github/workflows/bench-nightly.yml`) that runs `cargo bench --features
  encryption` on master with criterion baseline caching between runs.
  Enables regression detection between releases without manual re-benching.
  Source: Pareto plan D5.

---

## CI / process

- `[x]` **jscpd duplication gate in CI.** A `jscpd` step in CI that fails
  on code duplication above 2% (current baseline: ~1%). Configured via
  `.jscpd.json`, wired into both `ci.yml` and `scripts/verify-gate.sh`.
  Source: Pareto plan D6.

---

## See also

- [ROADMAP.md](ROADMAP.md) — long-term direction: async I/O, envelope v2
  (streaming CBOR early-stop, Blake3 checksum, compression negotiation,
  metadata block, cipher auto-detection), streaming cipher, second
  `SegmentStore` impl.
- [CHANGELOG.md](CHANGELOG.md) — shipped work.
- [FEATURES.md](FEATURES.md) — current capability inventory by status.
- [`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
  — full rationale for the envelope v2 deferrals.
- [`docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md`](docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md)
  — Pareto plan and addendum covering the perf batch that shipped
  (tuning guide, Vec recycling, background-flush pattern example).
- [`docs/planning/2026-08-11_05-12_post-v0-6-0-pareto-code-quality-ci-hardening.md`](docs/planning/2026-08-11_05-12_post-v0-6-0-pareto-code-quality-ci-hardening.md)
  — the post-v0.6.0 Pareto plan (executed 2026-08-11, all phases P0–P6 complete).
