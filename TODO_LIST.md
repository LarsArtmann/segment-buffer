# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher) live in
[ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress.

---

## Gate & CI

- `[ ]` **Add `check-changelog-links.sh` to `.github/workflows/ci.yml`.** The
  script is wired into the local `scripts/verify-gate.sh` gate but CI does
  not run it — a split brain where the local gate is stricter than CI. If a
  CHANGELOG link breaks, CI stays green and the breakage ships. Effort:
  ~15min (add a job step mirroring the local gate block). Source:
  `docs/status/2026-08-04_00-07_changelog-links-gate-wiring-and-self-review.md`
  item f.2.

- `[ ]` **Add `set -euo pipefail` to `scripts/verify-gate.sh`.** The
  orchestrator currently uses only `set -u`; the sub-scripts already use
  `pipefail`. A silently failing pipeline in the orchestrator could mask
  errors. Effort: ~5min (verify no intentional non-zero exits break under
  `set -e`). Source: `docs/status/2026-08-04_00-07_*` item f.16.

- `[ ]` **Audit all `scripts/*.sh` for the `MAPFILE` vs `mapfile` issue.**
  `check-changelog-links.sh` had uppercase `MAPFILE` (not a builtin on this
  bash build) — dead code that had never run. If one script had it, others
  might too. Effort: ~10min. Source: `docs/status/2026-08-04_00-07_*` item
  f.9.

- `[ ]` **Make the `sed -n '2,NNp'` help-range in `verify-gate.sh`
  self-maintaining.** The `--help` output hardcodes a line-number range
  (`2,22p`) that drifts on every header edit. Compute the range dynamically
  from the header delimiter instead. Effort: ~15min. Source:
  `docs/status/2026-08-04_00-07_*` item f.10.

---

## Testing

- `[ ]` **Property test: arbitrary `flush` + `delete_acked` sequences →
  `stats().segment_count` always matches `count_disk_segments(dir)`.** The
  extended `sync_disk_bytes_matches_actual_disk_usage` property test checks
  reconciliation after sync, but no property test exercises the incremental
  counter across arbitrary flush/delete interleavings. Effort: ~1h. Source:
  `docs/status/2026-08-04_00-20_*` item f.2.

- `[ ]` **Loom test: `segment_count` consistency under concurrent `flush` +
  `delete_acked`.** The atomic is `Relaxed`-ordered (correct for an
  approximate metric), but a concurrent flush+delete could produce a
  momentary `segment_count` that doesn't match disk. Prove it never
  underflows past 0, or document that it can momentarily and
  `sync_disk_bytes` recalibrates. Effort: ~2h. Source:
  `docs/status/2026-08-04_00-20_*` item f.3.

- `[ ]` **Document the `segment_count` underflow contract.** If
  `delete_acked` is called when files have been externally removed (so
  `deleted` > actual segment_count atomic value), `fetch_sub` wraps to a
  huge `u64`. The code is self-healing via `sync_disk_bytes`, but the
  underflow behavior should be documented in the field's doc comment.
  Effort: ~15min. Source: `docs/status/2026-08-04_00-20_*` item f.4.

- `[ ]` **`segment_count` assertion in the `append_all` auto-flush test.**
  `append_all` calls `flush()` internally when the threshold is crossed, so
  it goes through the same `fetch_add(1)` codepath — but no test explicitly
  asserts `segment_count` after an `append_all`-triggered auto-flush.
  Effort: ~10min. Source: `docs/status/2026-08-04_00-20_*` item f.7.

- `[ ]` **Clean up the `read_from_concurrent_delete_acked` loom test
  sentinel.** The final assertion uses `id: 99` as a cache-invalidation
  sentinel but doesn't filter it from the assertion — the test data is
  slightly messy. Filter it or use a non-item invalidation mechanism.
  Effort: ~15min. Source: `docs/status/2026-08-04_00-13_*` item f.10.

- `[ ]` **Investigate pre-encoded `MockStore` for loom runtime
  optimization.** The loom suite doubled to ~220s after adding `read_from`
  tests (CBOR+zstd decode per schedule step). A `MockStore` that stores
  pre-encoded bytes and skips the encode pipeline might cut the cost to
  ~120s without losing schedule fidelity. Effort: investigation + ~2h if
  tractable. Source: `docs/status/2026-08-04_00-13_*` item f.3.

- `[ ]` **Loom test for `scan_segments` + `recover` interleaving.** Recovery
  seeds the cache directly; if a concurrent `read_from` sees the
  pre-recovery cache state, it could serve stale data. Not yet covered.
  Effort: ~2h. Source: `docs/status/2026-08-04_00-13_*` item f.4.

- `[ ]` **Property test for `for_each_from` under concurrent `flush`.** The
  lending iterator has the same Phase 1/Phase 2 gap as `read_from` but a
  different code path. Effort: ~1h. Source: `docs/status/2026-08-04_00-13_*`
  item f.14.

- `[ ]` **Concurrent property test for `delete_acked + flush`
  interleaving.** Both mutations racing the reader at once (currently only
  single-mutation races are property-tested). Effort: ~1h. Source:
  `docs/status/2026-08-04_00-13_*` item f.15.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, and the `iter_from` / `open_with_report` code blocks all need a
  human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15min. _(User action — requires a
  browser, not a code change.)_

---

## Features

- `[ ]` **Per-segment size distribution for tuning.** A size summary (e.g.
  p50/p90/max segment size) would help callers tune `FlushPolicy::Batch(N)`
  against their throughput-vs-segment-count tradeoff. **Design question:**
  maintain a running summary in the mutex (O(1) read, but adds invariant
  surface area that must stay synced across flush + delete), or add a separate
  `segment_size_stats()` method that scans on demand (like `sync_disk_bytes` —
  simpler, but O(n\_segments) per call)? **Un-defer when:** a consumer
  (monitor365) reports needing segment-size distribution for batch tuning.
  Source: moved from ROADMAP.md.

---

## Design decisions deferred

- `[ ]` **Health-check primitive — needs a design decision before any code.** A `fn health(&self) -> Result<HealthReport>` that probes directory writability, lock validity, and disk space. **The design question that must be answered first:** _what does a caller learn from `health()` that they cannot learn from `stats()` + a trial `append()`?_ Three candidate designs, each with a reason it might be Verschlimmbessern:

  | Design                            | What it does                                              | Why it might make things worse                                                                                                                                                                                                    |
  | --------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `health()` wraps `stats()`        | Returns pressure, seq, disk bytes                         | **Redundant.** `stats()` already returns this. Adding a method that repackages it is API bloat with zero new information.                                                                                                         |
  | `health()` writes a sentinel file | Write + delete a `.healthcheck` file to probe writability | **Actively harmful on a near-full filesystem.** The write itself can fail (ENOSPC), and writing to a disk you're checking is healthy can worsen the condition.                                                                    |
  | `health()` checks free disk space | Statfs/GetDiskFreeSpace to report free bytes              | **Platform dependency.** Needs a new crate (`nix`, `winapi`, or `fs2`) for a feature that `store_pressure()` already approximates. Cross-platform free-space queries have subtle differences (available vs free vs total blocks). |

  **Current verdict:** defer until a concrete consumer needs it. The canonical health check today is: call `stats()` for pressure, call `append()` with a trivial item and check for `Err` — the error is already typed (`SegmentError::Io` with `IoSite`). If a consumer needs lock-validity checking, the `Drop` impl already panics if the lock file was tampered with; an explicit probe adds little. **Un-defer when:** a real deployment reports that `stats() + trial append` is insufficient to detect a degraded state.

- `[ ]` **Document panic-free guarantee as a public API contract?** The
  strict lint architecture (on master, unreleased) makes library code
  provably free of `unwrap()`, `expect()`, direct indexing, and string
  slicing — enforced by `pedantic` + `nursery` + restriction lints at
  `deny` in `Cargo.toml [lints.clippy]`. The only panic path is the
  documented `for_each_from` re-entrancy guard. **The design question:**
  is making "panic-free public API" an explicit documented guarantee a
  selling point worth the commitment, or should it stay an internal
  quality bar? A public guarantee is marketable but creates a maintenance
  contract. **Un-defer when:** the crate is pitched to a new audience
  (blog post, conference talk) where the guarantee is a differentiator,
  or a consumer asks "can this panic?"

- `[ ]` **`mtime_supported == false` scan-cache gap — fix or formally
  accept.** The scan-cache TOCTOU fix (`dc7ea7a`) only helps on filesystems
  where `mtime` advances (`mtime_supported == true`: ext4/xfs/tmpfs/APFS/
  NTFS — the common case). On coarse-granularity filesystems where the
  open-time probe reports `false`, the cache relies solely on explicit
  `invalidate_scan_cache` and the mid-scan-rename edge is not covered.
  This is documented honestly in DOMAIN_LANGUAGE.md. **The design
  question:** invest in a second validation mechanism for that path, or
  formally accept the documented limitation? **Un-defer when:** a consumer
  reports operating on a filesystem where `mtime_supported == false`.

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
