# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher) live in
[ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress.

---

## Testing

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, Mermaid diagram, and the `iter_from` / `open_with_report` code blocks
  all need a human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15 min. _(User action — requires a
  browser, not a code change.)_

---

## Design decisions deferred

- `[ ]` **`segment_count` type consistency: `u64` vs `usize`.**
  `BufferStats::segment_count` is `u64` (matching `approx_disk_bytes`);
  `RecoveryReport::segment_count` is `usize`. Both are correct for their
  context, but the inconsistency should be noted and either documented or
  reconciled. **Un-defer when:** the next release that touches either
  struct. Source: `docs/status/2026-08-04_00-20_*` item g.1.

---

## Resolved decisions (for reference)

These were open design questions that have been settled. Kept here briefly so
the rationale is discoverable; the authoritative record is in CHANGELOG.md
and the status reports cited.

- **Health-check primitive — DEFER (2026-08-04).** All three candidate designs
  are Verschlimmbessern (redundant, disk-harmful, platform dependency). The
  canonical health check is `stats()` for pressure plus a trial `append()` +
  `flush()` to probe writability. **Un-defer when:** a real deployment reports
  that `stats()` + `flush()` is insufficient to detect a degraded state. Source:
  `docs/status/2026-08-04_01-12_*`.

- **Panic-free public API — SHIPPED (2026-08-04).** The re-entrancy deadlock
  was eliminated at the root (`for_each_from` no longer holds the mutex across
  the callback). Zero `panic!` paths in library code. See CHANGELOG `[Unreleased]
→ Changed`.

- **`mtime_supported == false` scan-cache gap — FORMALLY ACCEPTED
  (2026-08-04).** The single-process invariant already forbids external
  directory mutation, so the `mtime` guard is defense-in-depth, not a primary
  guarantee. **Re-open if:** a consumer reports operating on a filesystem
  where `mtime_supported == false`. Source: `docs/status/2026-08-04_01-12_*`.

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
