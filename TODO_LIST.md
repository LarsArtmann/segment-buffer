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

- `[ ]` **Deterministic Barrier-based regression test for the scan-cache
  TOCTOU.** The scan-cache mtime-ordering fix (`dc7ea7a`) is validated 40×
  in release but not via a deterministic `std::sync::Barrier` test that
  forces the exact `scan → rename → scan-returns-stale` interleaving. A
  deterministic test _proves_ the fix rather than _supporting_ it. Effort:
  ~2h. Source: `docs/status/2026-08-02_15-50_scan-cache-toctou-fix-and-gate.md`
  item b.1.

- `[ ]` **Loom coverage for `scan_segments`.** The 9 loom tests cover the
  in-memory hot path and the `delete_acked` + `append` interleaving, but
  none exercise the scan cache. The `MockStore` injected via
  `open_with_store` could in principle stub `scan()` to return a controlled
  segment list, making the cache populate/invalidate interleaving
  exhaustively checkable. Effort: investigation + ~3h if tractable. Source:
  `docs/status/2026-08-02_15-50_scan-cache-toctou-fix-and-gate.md` item b.1.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, and the `iter_from` / `open_with_report` code blocks all need a
  human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15min. _(User action — requires a
  browser, not a code change.)_

- `[ ]` **Wire `check-changelog-links.sh` into `scripts/verify-gate.sh`.**
  The script exists (`scripts/check-changelog-links.sh`) but is not part
  of the automated gate. A check that isn't wired into the gate rots.
  Effort: ~10min.

- `[ ]` **Document `pending_count()` vs `unflushed` distinction** in
  rustdoc. `pending_count()` returns the total backlog (disk + memory);
  the internal `unflushed: Vec<T>` is in-memory only. The distinction is
  not obvious from the method name. Adding a doc note (option (c) from the
  06-15 report's Q3) is the safest fix — no API change. Effort: ~15min.

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
