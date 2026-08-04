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

- `[x]` **Parametrize the percentile property test over `pct in 0u32..=100`.**
  The nearest-rank formula is currently proven for exactly p50 and p90 — the
  two values the API happens to expose. A parametrized test would prove it for
  _all_ percentiles and future-proof for `p99_bytes`. Effort: ~20 min. Source:
  `docs/status/2026-08-04_01-01_*` item f.5. **Done** (2026-08-04):
  `percentile_of_sorted_matches_nearest_rank_for_all_pct` in
  `src/property_tests.rs`.

- `[x]` **Direct unit test of `percentile_of_sorted` edge cases** (empty input,
  `pct=0`, `pct=100`, `n=1`). The private helper is currently only tested
  indirectly via `segment_size_stats`. A direct test makes the nearest-rank
  contract visible. Effort: ~10 min. Source: `docs/status/2026-08-04_01-01_*`
  item f.6. **Done** (2026-08-04): five tests in `src/tests.rs` covering
  empty, pct=0, pct=100, n=1, and monotonicity.

- `[x]` **Encrypted-segment `segment_size_stats` test.** The code path is
  identical regardless of encryption (`segment_size` reads
  `metadata().len()`), so this is belt-and-braces rather than a correctness
  gap. The crate has encrypted variants of other tests; this one is missing
  for consistency. Effort: ~10 min. Source: `docs/status/2026-08-04_01-01_*`
  item f.7. **Done** (2026-08-04):
  `segment_size_stats_works_with_encrypted_segments` in `src/tests.rs`.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, Mermaid diagram, and the `iter_from` / `open_with_report` code blocks
  all need a human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15 min. _(User action — requires a
  browser, not a code change.)_

- `[x]` **`examples/segment_tuning.rs`** — a runnable demo showing
  `segment_size_stats()` used to adjust `FlushPolicy::Batch(N)` based on
  observed p50/max. The feature's stated purpose (tuning) has no example;
  the crate has 13 examples for other use cases. Effort: ~30 min. Source:
  `docs/status/2026-08-04_01-01_*` item f.4. **Done** (2026-08-04):
  `examples/segment_tuning.rs` — three-phase tuning loop (baseline → sweep →
  recommendation) with a configurable target segment-size window.

- `[x]` **Document why `segment_size_stats` is absent from the loom suite.**
  It adds no mutex concurrency surface (pure query reusing the
  already-covered `scan_segments` path), but the justification should be
  noted in a comment in `tests/loom.rs` or in AGENTS.md. Effort: ~5 min.
  Source: `docs/status/2026-08-04_01-01_*` item f.8. **Done** (2026-08-04):
  paragraph added to `tests/loom.rs` module docs → "What this does NOT
  cover".

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
