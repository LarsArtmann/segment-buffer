# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher) live in
[ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, and the `iter_from` / `open_with_report` code blocks all need a
  human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15min. _(User action — requires a
  browser, not a code change.)_

---

## Design decisions deferred

- `[ ]` **Health-check primitive — needs a design decision before any code.** A `fn health(&self) -> Result<HealthReport>` that probes directory writability, lock validity, and disk space. **The design question that must be answered first:** _what does a caller learn from `health()` that they cannot learn from `stats()` + a trial `append()`?_ Three candidate designs, each with a reason it might be Verschlimmbessern:

  | Design                            | What it does                                              | Why it might make things worse                                                                                                                                                                                                    |
  | --------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `health()` wraps `stats()`        | Returns pressure, seq, disk bytes                         | **Redundant.** `stats()` already returns this. Adding a method that repackages it is API bloat with zero new information.                                                                                                         |
  | `health()` writes a sentinel file | Write + delete a `.healthcheck` file to probe writability | **Actively harmful on a near-full filesystem.** The write itself can fail (ENOSPC), and writing to a disk you're checking is healthy can worsen the condition.                                                                    |
  | `health()` checks free disk space | Statfs/GetDiskFreeSpace to report free bytes              | **Platform dependency.** Needs a new crate (`nix`, `winapi`, or `fs2`) for a feature that `store_pressure()` already approximates. Cross-platform free-space queries have subtle differences (available vs free vs total blocks). |

  **DECISION (2026-08-04): DEFER.** No `health()` primitive — all three candidate designs are Verschlimmbessern (redundant, disk-harmful, or a platform dependency for something `store_pressure()` already approximates). The canonical health check is `stats()` for pressure plus a trial `append()`; note a single `append()` below the flush threshold never hits disk, so add an explicit `flush()` to probe writability. (The `Drop` impl is best-effort — it does **not** panic on lock tampering; lock loss is not surfaced as a health signal today.) **Un-defer when:** a real deployment reports that `stats()` + `flush()` is insufficient to detect a degraded state.

- `[x]` **Panic-free public API — shipped (2026-08-04).** The re-entrancy
  deadlock was eliminated at the root: `for_each_from` no longer holds the
  buffer mutex across the user callback (in-memory pending items are
  snapshotted under the lock, then the lock is released before the callback).
  The `assert_not_reentered` guard, `iteration_in_progress` flag, and
  `IterationGuard` RAII type are deleted — there are now zero `panic!` paths
  in library code. Re-entrant calls from inside a `for_each_from` callback
  (`append`, `stats`, `delete_acked`, …) are safe instead of panicking. The
  quality posture is documented in `README.md` § Guarantees (as a quality
  bar, not a load-bearing API contract). Tradeoff: the in-memory tail is now
  cloned once (bounded by `limit`), so `for_each_from` is no longer ~21×
  faster than `read_from` on pure in-memory reads — they are now roughly
  equal.

- `[x]` **`mtime_supported == false` scan-cache gap — FORMALLY ACCEPTED
  (2026-08-04).** The scan-cache TOCTOU fix only helps where `mtime` advances
  (ext4/xfs/tmpfs/APFS/NTFS). On coarse-granularity filesystems the cache
  relies solely on `invalidate_scan_cache`, leaving external mid-scan rename
  uncovered. **Accepted because** the single-process invariant already forbids
  external directory mutation, so the `mtime` guard is defense-in-depth against
  contract violations, not a primary guarantee. The limitation stays documented
  in `docs/DOMAIN_LANGUAGE.md`. Re-open if a consumer reports operating on a
  filesystem where `mtime_supported == false`.

- `[ ]` **`segment_count` type consistency: `u64` vs `usize`.**
  `BufferStats::segment_count` is `u64` (matching `approx_disk_bytes`);
  `RecoveryReport::segment_count` is `usize`. Both are correct for their
  context, but the inconsistency should be noted and either documented or
  reconciled. **Un-defer when:** the next release that touches either
  struct. Source: `docs/status/2026-08-04_00-20_*` item g.1.

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
