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

- `[ ]` **Edge-case tests for `BatchOrIntervalMin`.** The existing pure
  decision tests and the exhaustive proptest cover the main formula, but
  three boundary conditions have no explicit coverage: `min_batch == 0`
  (degenerates to `BatchOrInterval`), `max_interval == interval` (gated
  interval is unreachable), `min_batch == batch_size` (interval trigger is
  unreachable). Each is a one-liner assertion against `should_flush`.
  Effort: ~15min.

- `[ ]` **Fuzz target for flush-policy parameters.** The fuzz suite covers
  corrupted read, recovery, filename parsing, envelope, and `append_all`.
  A target that randomizes `(batch_size, min_batch, interval,
max_interval, append_pattern)` and asserts the flush invariant could
  catch edge cases in the boolean logic of `should_flush`. Effort: ~1h.

- `[ ]` **Concurrency test using `BatchOrIntervalMin` under multi-writer
  load.** The existing concurrency stress tests use `FlushPolicy::Manual`
  (per AGENTS.md rule 7). A test with `BatchOrIntervalMin` under N writers
  would verify the new policy's `should_flush` arm is safe under
  contention. Effort: ~30min.

---

## Documentation

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, and the `iter_from` / `open_with_report` code blocks all need a
  human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item; widened surface from the v0.5.4 Status
  section update._ Effort: ~15min. _(User action — requires a browser,
  not a code change.)_

- `[ ]` **Update CONTRIBUTING.md lint commands.** CONTRIBUTING.md still
  shows `cargo clippy --all-targets -- -D warnings` without mentioning the
  declarative `[lints.clippy]` section in Cargo.toml. The commands still
  work (Cargo `[lints]` is additive to `-D warnings`), but contributors
  should know lints are also enforced declaratively. Add a "Lint
  architecture" subsection or a note pointing to AGENTS.md. Effort: ~10min.

- `[ ]` **Document `last_flush` initialization timing.** `last_flush` is
  set at `Instant::now()` in `BufferInner` construction — BEFORE
  `recover()` runs. Under a slow recovery (large segment count), the first
  interval-triggered flush fires earlier than expected. Harmless (a
  slightly eager first flush) but surprising with very short intervals.
  Document in `FlushPolicy` rustdoc or DOMAIN_LANGUAGE.md. Effort: ~10min.

---

## API ergonomics

- `[ ]` **`Display` impl for `FlushPolicy`.** Currently there is no
  `Display` impl, so logging a config dumps the `Debug` representation
  (verbose struct format). A `Display` impl would produce clean
  one-liners like `Batch(1000)` or
  `BatchOrIntervalMin { batch: 1000, min: 10, every: 5s, max: 30s }`.
  Effort: ~20min.

- `[ ]` **Standalone example for `BatchOrIntervalMin`**
  (`examples/batch_or_interval_min.rs`). The PERFORMANCE.md callout
  covers the builder snippet, but no runnable example demonstrates the
  variant in a realistic scenario (low-throughput producer suppressing
  tiny segments). Effort: ~30min.

---

## CI / release tooling

- `[ ]` **Make `publish.yml` idempotent.** The automated publish workflow
  fails with "crate already exists" when a manual `cargo publish` has
  already landed the version (this happened during the v0.5.4 release).
  Add a pre-step that checks `cargo info segment-buffer@$VERSION` and
  exits 0 (success) if the version already exists, so future
  double-publishes produce a green CI run instead of a red one. Effort:
  ~15min.

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
