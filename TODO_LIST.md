# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher) live in
[ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md); current capabilities in
[FEATURES.md](FEATURES.md).

Settled design decisions (health-check primitive rejected, panic-free API
shipped, `mtime` gap formally accepted, `segment_count` type documented) are
recorded in [ROADMAP.md](ROADMAP.md) § Non-goals,
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
  Source: AGENTS.md § Durability model, `docs/status/2026-08-04_04-15_*`.

---

## Testing & concurrency coverage

- `[ ]` **Add loom coverage for the `for_each_from` snapshot-then-release-lock
  pattern.** The v0.5.5 panic-free refactor introduced a new concurrency surface
  (snapshot the in-memory pending window under the lock, release the lock, then
  invoke the callback). It is covered statistically today by the stress test
  `concurrency_4_writers_1_reader_10k_events`; an exhaustive loom test would
  prove every interleaving. Source: `docs/status/2026-08-04_04-15_*`.

- `[ ]` **Add a property test for `compute_store_pressure`.** The extracted
  helper (`bytes / max` clamped to `[0, 1]`) is a pure function and trivially
  testable, yet has no dedicated property test. Verify the `max == 0` early
  return, the clamp at `1.0`, and monotonicity in `bytes`. Source:
  `docs/status/2026-08-04_04-15_*`.

- `[ ]` **Add a fuzz target for `for_each_from`.** Arbitrary `start` + `limit`
  with concurrent mutations (flush / delete_acked) to complement the existing
  `fuzz_append_all` and `fuzz_recovery` targets. Source:
  `docs/status/2026-08-04_04-15_*`.

---

## Code quality

- `[ ]` **Convert the 4 remaining `"p-{i}"` `PropItem` constructions** in
  `src/property_tests.rs` to the `prop_item(i)` helper. No test asserts on the
  payload string content — only on ids and counts — so the conversion is safe
  and removes the last inline-construction noise. Source:
  `docs/status/2026-08-04_04-15_*`.

- `[ ]` **Derive `PartialEq` for `SegmentConfig`.** Would make test assertions
  on config equality direct instead of field-by-field. Verify the cipher field
  (`Arc<dyn SegmentCipher + Send + Sync>`) does not block the derive (it should
  not — `Arc` of a non-`PartialEq` trait object does, so this may need
  narrowing to the builder or a test-only comparison helper). Source:
  `docs/status/2026-08-04_04-15_*`.

---

## Documentation (standing item)

- `[ ]` **Visually verify README rendering** on GitHub, docs.rs, and a
  narrow viewport (mobile-width). The ToC, Status block, Cargo features
  table, Mermaid diagram, and the `iter_from` / `open_with_report` code blocks
  all need a human eye — lychee catches link and anchor drift, not rendering
  regressions. _Standing item._ Effort: ~15 min. _(User action — requires a
  browser, not a code change.)_

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
