# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher, parallel flush
workers, incremental `pedantic`/`nursery` lint adoption) live in
[ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (recent entries
stay until the next CHANGELOG cut, then move out).

---

## Testing

- `[x]` **Edge-case tests for `BatchOrIntervalMin`.** Three boundary
  conditions now covered: `min_batch == 0` (always flushes at interval),
  `max_interval == interval` (min_batch irrelevant), `min_batch ==
batch_size` (interval arm reduces to batch arm). _(2026-08-02)_

- `[x]` **Fuzz target for flush-policy parameters.** `fuzz_flush_policy.rs`
  exercises `should_flush` over arbitrary parameter combinations. Registered
  in `fuzz/Cargo.toml`. _(2026-08-02)_

- `[x]` **Concurrency test using `BatchOrIntervalMin` under multi-writer
  load.** `concurrency_batch_or_interval_min_4_writers_10k_events`:
  4 writers × 2 500 events with auto-flush at `batch_size=1000`. _(2026-08-02)_

- `[x]` **Property tests for the consistency model.** Formal proptest
  assertions for the two documented race windows (concurrent `delete_acked`
  spurious Io; concurrent `flush` transient gap). Five property tests added:
  three deterministic (surviving-items-correct-after-delete,
  disk-memory-split-correctness, all-visible-after-flush) and two concurrent
  (delete-acked race, flush race) with generated parameters. The deterministic
  tests make the invariants machine-checkable for every generated state; the
  concurrent tests broaden coverage beyond the fixed-parameter stress tests.
  _(2026-08-02)_

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, and the `iter_from` / `open_with_report` code blocks all need a
  human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item; widened surface from the v0.5.4 Status
  section update._ Effort: ~15min. _(User action — requires a browser,
  not a code change.)_

- `[x]` **Update CONTRIBUTING.md lint commands.** Added "Lint architecture"
  subsection documenting the two-tier Clippy strategy. Clippy commands updated
  with `-A clippy::pedantic`. _(2026-08-02)_

- `[x]` **Document `last_flush` initialization timing.** Added "Timing note"
  to `Interval` and `BatchOrInterval` variant docs: the interval clock starts
  at `open()`, not at the first `append()`. _(2026-08-02)_

- `[x]` **`BatchOrIntervalMin` in tradeoffs matrix.** Added to
  `docs/DOMAIN_LANGUAGE.md` tradeoffs table. _(2026-08-02)_

- `[x]` **Release runbook in AGENTS.md.** Step-by-step procedure with 11
  steps including CI-green check, tag-before-push ordering, and GitHub release
  API workaround. _(2026-08-02)_

- `[x]` **CHANGELOG link-validation script.**
  `scripts/check-changelog-links.sh` validates GitHub tag URLs. _(2026-08-02)_

- `[x]` **26 historical status reports archived.** Resolved July reports
  moved to `docs/status/archived/`. 6 current reports remain. _(2026-08-02)_

---

## API ergonomics

- `[x]` **`Display` impl for `FlushPolicy`.** All variants produce stable,
  parseable output (`batch(256)`, `interval(5s)`,
  `batch_or_interval_min(batch=256, min=10, interval=5s, max=60s)`,
  `manual`). Snapshot test included. _(2026-08-02)_

- `[x]` **Standalone example for `BatchOrIntervalMin`**
  (`examples/batch_or_interval_min.rs`). Demonstrates burst-then-drip
  scenario showing tiny-segment suppression. _(2026-08-02)_

---

## CI / release tooling

- `[x]` **Make `publish.yml` idempotent.** Added crates.io API pre-check:
  queries the version endpoint, skips publish if HTTP 200 (already exists).
  Prevents red CI on workflow re-runs. _(2026-08-02)_

- `[x]` **Cargo.lock drift check in CI.** Added `cargo fetch --locked` step
  to CI that catches unintended transitive dep bumps. _(2026-08-02)_

- `[x]` **`pedantic` Clippy at `warn` level.** Added to `[lints.clippy]` in
  Cargo.toml. ~62 warnings visible during local `cargo clippy`. CI commands
  include `-A clippy::pedantic` to suppress in the gate. `error.rs` is
  pedantic-clean. _(2026-08-02)_

- `[x]` **`bacon` in Nix devShell.** Live clippy feedback during development.
  _(2026-08-02)_

- `[x]` **`FlushPolicy` fuzz target exposed via `fuzz_hooks`.** Added
  `should_flush` wrapper and `FlushPolicy` re-export to `fuzz_hooks` module.
  _(2026-08-02)_

---

## Quality backlog

- `[~]` **Incremental `pedantic` migration.** `error.rs` is pedantic-clean
  (11 warnings fixed — all missing backticks). Remaining modules: `lib.rs`
  (~30 warnings), `segment.rs` (~10), `store.rs` (~5), `cipher.rs` (~6).
  Most are missing-backticks and missing-`#[must_use]`. Effort: ~2h total.

- `[ ]` **Audit benchmarks/examples for lint posture.** Benches use
  `unwrap()` by convention (test-adjacent code, panicking is the correct
  failure mode). Examples use `?` + `Box<dyn Error>` in `main()` and
  `.expect()` in thread-join sites. No action needed — the two-tier lint
  architecture correctly excludes these targets from the strict library
  denies. Document this convention in CONTRIBUTING.md if it becomes
  confusing. Effort: ~5min (doc-only).

---

## Design decisions deferred

- `[ ]` **Health-check primitive — needs a design decision before any code.** A `fn health(&self) -> Result<HealthReport>` that probes directory writability, lock validity, and disk space. **The design question that must be answered first:** _what does a caller learn from `health()` that they cannot learn from `stats()` + a trial `append()`?_ Three candidate designs, each with a reason it might be Verschlimmbessern:

  | Design                            | What it does                                              | Why it might make things worse                                                                                                                                                                                                    |
  | --------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `health()` wraps `stats()`        | Returns pressure, seq, disk bytes                         | **Redundant.** `stats()` already returns this. Adding a method that repackages it is API bloat with zero new information.                                                                                                         |
  | `health()` writes a sentinel file | Write + delete a `.healthcheck` file to probe writability | **Actively harmful on a near-full filesystem.** The write itself can fail (ENOSPC), and writing to a disk you're checking is healthy can worsen the condition.                                                                    |
  | `health()` checks free disk space | Statfs/GetDiskFreeSpace to report free bytes              | **Platform dependency.** Needs a new crate (`nix`, `winapi`, or `fs2`) for a feature that `store_pressure()` already approximates. Cross-platform free-space queries have subtle differences (available vs free vs total blocks). |

  **Current verdict:** defer until a concrete consumer needs it. The canonical health check today is: call `stats()` for pressure, call `append()` with a trivial item and check for `Err` — the error is already typed (`SegmentError::Io` with `IoSite`). If a consumer needs lock-validity checking, the `Drop` impl already panics if the lock file was tampered with; an explicit probe adds little. **Un-defer when:** a real deployment reports that `stats() + trial append` is insufficient to detect a degraded state (e.g., lock file deleted by an external process while the buffer is open).

- `[ ]` **Document panic-free guarantee as a public API contract?** The
  two-tier lint architecture (unreleased, on master) makes library code
  provably free of `unwrap()`, `expect()`, direct indexing, and string
  slicing — enforced by `#![deny(...)]` in `src/lib.rs`. The only panic
  path is the documented `for_each_from` re-entrancy guard. **The design
  question:** is making "panic-free public API" an explicit documented
  guarantee a selling point worth the commitment, or should it stay an
  internal quality bar? A public guarantee is marketable but creates a
  maintenance contract. **Un-defer when:** the crate is pitched to a new
  audience (blog post, conference talk) where the guarantee is a
  differentiator, or a consumer asks "can this panic?"

---

## See also

- [ROADMAP.md](ROADMAP.md) — long-term direction: async I/O, envelope v2
  (streaming CBOR early-stop, Blake3 checksum, compression negotiation,
  metadata block, streaming cipher), second `SegmentStore` impl,
  incremental `pedantic`/`nursery` lint adoption.
- [CHANGELOG.md](CHANGELOG.md) — shipped work.
- [`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
  — full rationale for the envelope v2 deferrals.
- [`docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md`](docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md)
  — Pareto plan and addendum covering the perf batch that shipped
  (tuning guide, Vec recycling, background-flush pattern example).
- [`docs/status/2026-08-02_05-03_namtao-rust-learnings-and-strict-lint-adoption.md`](docs/status/2026-08-02_05-03_namtao-rust-learnings-and-strict-lint-adoption.md)
  — source of the lint adoption items and the panic-free guarantee
  question.
