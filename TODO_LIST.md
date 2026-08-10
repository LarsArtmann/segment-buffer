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

## Testing & concurrency coverage

- `[ ]` **Add loom coverage for the `for_each_from` snapshot-then-release-lock
  pattern.** The v0.5.5 panic-free refactor introduced a new concurrency surface
  (snapshot the in-memory pending window under the lock, release the lock, then
  invoke the callback). It is covered statistically today by the stress test
  `concurrency_4_writers_1_reader_10k_events`; an exhaustive loom test would
  prove every interleaving. Source: `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Add a loom test for `iter_from`.** The loom suite covers `read_from`
  (scan-cache tests) and `for_each_from` (indirectly via `delete_acked` tests),
  but the materialising `iter_from` path has no dedicated loom proof. Source:
  `docs/status/archived/2026-08-04_02-48_*`.

- `[ ]` **Add a property test for `compute_store_pressure`.** The extracted
  helper (`bytes / max` clamped to `[0, 1]`) is a pure function and trivially
  testable, yet has no dedicated property test. Verify the `max == 0` early
  return, the clamp at `1.0`, and monotonicity in `bytes`. Source:
  `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Add a property test for `publish_disk_stats` correctness.** Verify the
  atomic counters (`approx_disk_bytes`, `segment_count`) match reality after
  `sync_disk_bytes` and `recover` across arbitrary append/flush/delete sequences.
  Source: `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Add a property test for `delete_acked` idempotency under concurrent
  `append`.** A loom test (`delete_acked_idempotent_under_concurrent_append`)
  exists; a property-test complement covering larger schedules would strengthen
  the proof. Source: `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Add percentile edge-case property tests** for `percentile_of_sorted`:
  `n=0` (empty slice) and duplicate values (ties). The parametrized test
  (`percentile_of_sorted_matches_nearest_rank_for_all_pct`) starts at `n=1` and
  uses distinct ascending values; real segment sizes can have ties. Source:
  `docs/status/archived/2026-08-04_01-53_*`.

- `[ ]` **Add a stress test for `segment_size_stats` under concurrent `flush` +
  `delete_acked`.** Currently only tested sequentially. A concurrent stress test
  would prove the `O(n_segments)` scan is safe under mutation. Source:
  `docs/status/archived/2026-08-04_01-53_*`.

- `[ ]` **Add a fuzz target for `for_each_from`.** Arbitrary `start` + `limit`
  with concurrent mutations (flush / delete_acked) to complement the existing
  `fuzz_append_all` and `fuzz_recovery` targets. Source:
  `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Add an XChaCha20 variant of the encrypted `segment_size_stats` test.**
  The test currently covers AES-GCM only; XChaCha20 is the recommended cipher for
  new buffers. Belt-and-braces parity, ~10 min. Source:
  `docs/status/archived/2026-08-04_01-53_*`.

---

## Code quality

- `[ ]` **Convert the 4 remaining `"p-{i}"` `PropItem` constructions** in
  `src/property_tests.rs` to the `prop_item(i)` helper. No test asserts on the
  payload string content — only on ids and counts — so the conversion is safe
  and removes the last inline-construction noise. Source:
  `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Derive `PartialEq` for `SegmentConfig`.** Would make test assertions
  on config equality direct instead of field-by-field. Verify the cipher field
  (`Arc<dyn SegmentCipher + Send + Sync>`) does not block the derive (it should
  not — `Arc` of a non-`PartialEq` trait object does, so this may need
  narrowing to the builder or a test-only comparison helper). Source:
  `docs/status/archived/2026-08-04_04-15_*`.

- `[ ]` **Extract a `seq_to_index(u64) -> usize` helper** for the
  `usize::try_from(x).unwrap_or(usize::MAX)` pattern repeated at 3 call sites
  in `read_from`/`for_each_from`. Source:
  `docs/status/archived/2026-08-02_16-43_*`.

- `[ ]` **Add `FlushPolicy::validate()` method.** Move the `debug_assert!`s for
  `min_batch <= batch_size` etc. into a reusable method callable from both the
  builder and `open()`. Source:
  `docs/status/archived/2026-08-02_06-15_*`.

- `[ ]` **Seal the `SegmentStore` trait.** The trait is reachable under the
  `loom` feature; the "not semver" claim currently relies on convention, not
  enforcement. Standard supertrait-in-private-module pattern. Source:
  `docs/status/archived/2026-08-04_04-15_*`.

---

## API ergonomics

- `[ ]` **Add `Display` impls for `DurabilityPolicy`, `BufferStats`, and
  `SegmentConfig`.** Improves error messages, logging, and debugging output.
  `FlushPolicy` already has `Display`; these types are `Debug`-only today.
  Source: `docs/status/archived/2026-08-02_06-15_*`,
  `docs/status/archived/2026-08-04_01-53_*`.

- `[ ]` **Add `#[doc(alias = "backlog")]` on `pending_count()`.** Improves
  discoverability — users searching for "backlog" in rustdoc land on the right
  method. Source: `docs/status/archived/2026-08-02_06-15_*`.

- `[ ]` **Add `#[must_use]` to the `BufferStats` struct.** The `stats()` method
  already has `#[must_use`, but the struct itself does not. ~2 min. Source:
  `docs/status/archived/2026-08-04_00-20_*`.

---

## Benchmarks

- `[ ]` **Add `bench_segment_size_stats`.** Quantify the `O(n_segments)` scan
  cost at 100 / 1k / 10k segments. No bench file for this exists today. Source:
  `docs/status/archived/2026-08-04_01-01_*`.

- `[ ]` **Add `bench_cipher` (encryption overhead vs no-cipher baseline).** The
  encryption path has never been benchmarked. No bench file measures AES-GCM /
  XChaCha20 overhead. Source:
  `docs/status/archived/2026-07-20_02-24_*`.

---

## Documentation

- `[ ]` **Add `batch_or_interval_min` and `segment_tuning` to the crate-level
  Examples table** in `src/lib.rs` (`# Examples` section). The table lists 12
  examples but omits the two newest (added in v0.5.4 / v0.5.5). Source:
  `docs/status/archived/2026-08-04_01-58_*`.

- `[ ]` **Add a `# Guarantees` section to the crate-level rustdoc.** The README
  has a `## Guarantees` section documenting the panic-free API; the crate-level
  rustdoc has `# Delivery guarantees` but no section about the panic-free
  contract. Source: `docs/status/archived/2026-08-04_01-12_*`.

- `[ ]` **Expand the FEATURES.md examples inventory.** The "Documentation &
  examples" table lists only 3 of 14 examples. Either list all or link to a
  directory listing. Source: `docs/status/archived/2026-08-04_01-58_*`.

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, Mermaid diagram, and the `iter_from` / `open_with_report` code blocks
  all need a human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15 min. _(User action — requires a
  browser, not a code change.)_

---

## CI / process

- `[ ]` **Audit CI vs local gate parity.** Enumerate every check in `ci.yml`
  and every check in `scripts/verify-gate.sh`, diff the two lists, document or
  fix every divergence. Source:
  `docs/status/archived/2026-08-04_00-07_*`.

- `[ ]` **Add clippy with full lint stack to the MSRV CI job.** Currently the
  MSRV job only runs `cargo check`, not `cargo clippy` with the full
  `[lints.clippy]` deny set. Source:
  `docs/status/archived/2026-08-04_01-03_*`.

- `[ ]` **Improve `check-changelog-links.sh` robustness.** Add rate-limit
  detection (HTTP 403 → warn + degrade gracefully) and `GITHUB_TOKEN` support
  (bumps GitHub API rate limit from 60/hour to 5000/hour). Source:
  `docs/status/archived/2026-08-04_00-07_*`.

- `[ ]` **Add `--list` and `--only=` options to `verify-gate.sh`.** `--list`
  prints all gate names without running; `--only=X,Y,Z` runs a subset for faster
  iteration. Source: `docs/status/archived/2026-08-04_00-07_*`.

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
