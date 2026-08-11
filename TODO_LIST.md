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

- `[ ]` **Extract `NopCipher` test helper.** The `NopCipher` struct + its
  `SegmentCipher` impl are duplicated verbatim 3× in `src/tests.rs` (at the
  `segment_config_partial_eq_*` test group, circa lines 1356 / 1375 / 1398).
  Extract to a shared helper at the top of the test module, like `prop_item`
  in `src/property_tests.rs`.
  Source: `docs/status/archived/2026-08-10_06-01_*` § e.5.

- `[ ]` **`format_bytes_human` edge-case unit tests.** The private
  `format_bytes_human` function in `src/lib.rs` (BufferStats Display) has no
  dedicated edge-case tests. Add tests for: `0` → `"0B"`, `1023` →
  `"1023B"`, `1024` → `"1.0KB"`, `1025` → `"1.0KB"`, `u64::MAX` → no panic.
  Source: `docs/status/archived/2026-08-10_06-01_*` § e.6.

- `[ ]` **Default-value regression guard for `compression_level`.** Add a
  test asserting `SegmentConfig::default().compression_level == 1` to catch
  accidental default drift. The matching test for
  `durability == DurabilityPolicy::Throughput` already exists
  (`durability_policy_default_is_throughput` in `src/tests.rs`).
  Source: `docs/status/archived/2026-08-10_15-51_*` § f.42.

---

## Benchmarks & performance

- `[ ]` **Run `bench_segment_size_stats` and `bench_cipher` at least once.**
  Both benchmarks were created in v0.5.6, compile-clean, and are registered in
  `Cargo.toml`, but have **never been executed**. Run each once to confirm
  they produce sensible output and don't panic at runtime.
  Source: `docs/status/archived/2026-08-10_06-51_*` § e.1–e.2.

- `[ ]` **Update `docs/PERFORMANCE.md` baseline snapshot.** The baseline
  (§ "Baseline snapshot") was recorded under v0.5.6 with the old level-3
  compression default. Since v0.5.7 the default is level 1 and the bench
  configs in `benches/support.rs` were updated to match. Re-run the criterion
  benchmarks under the current default and replace the stale snapshot.
  Source: `docs/status/archived/2026-08-10_15-51_*` § b.2.

- `[ ]` **Write compression-sweep analysis doc.** The TSV data exists at
  `docs/perf/2026-08-10_compression-level-sweep.tsv` (63 of 88 rows — missing
  json 20–22 and random 1–22; the conclusion is already clear from the
  captured rows) but no human-readable `.md` analysis accompanies it (unlike
  the 2026-07-21 scaling sweep). Document the level-1 recommendation with the
  sweep data.
  Source: `docs/status/archived/2026-08-10_15-51_*` § b.3.

- `[ ]` **Document concurrent-append and real-disk findings in
  `docs/PERFORMANCE.md`.** The `append_all` 3.6× advantage at 8 threads
  (mutex contention bottleneck) and the CPU-bound (not I/O-bound) finding
  from real-disk scaling tests are captured in the status report but not yet
  in PERFORMANCE.md.
  Source: `docs/status/archived/2026-08-10_15-51_*` § a.8–a.9.

---

## CI / process

- `[ ]` **Add remaining 5 fuzz targets to CI fuzz workflow.**
  `.github/workflows/fuzz.yml` matrix runs only `fuzz_corrupted_read` and
  `fuzz_recovery` (2 of 7). The other 5 (`fuzz_parse_filename`,
  `fuzz_envelope`, `fuzz_append_all`, `fuzz_flush_policy`,
  `fuzz_for_each_from`) have no CI coverage. Adding all 7 would ~3.5× the CI
  fuzz time (~6 min → ~21 min); consider a daily/weekly rotation split.
  Source: `docs/status/archived/2026-08-10_06-51_*` § e.2.

- `[ ]` **Fix "Update flake.lock" scheduled workflow.** The workflow at
  `.github/workflows/update-flake-lock.yml` has no `permissions:` block, so
  it uses the default read-only `GITHUB_TOKEN` and fails every run with `403
  Permission denied` when the bot tries to push. Either grant
  `contents: write` + `pull-requests: write` or disable the schedule.
  Source: `docs/status/archived/2026-08-10_09-23_*` § c.20.

- `[ ]` **Add Cargo.lock version-sync check to `scripts/verify-gate.sh`.** A
  new gate that asserts `Cargo.lock`'s `segment-buffer` version matches
  `Cargo.toml`'s `version` field. Would have caught the v0.5.7 publish
  failure (tagged and pushed without syncing `Cargo.lock`, causing the
  `publish.yml` workflow to fail on the first attempt).
  Source: `docs/status/archived/2026-08-10_16-32_*` § d.1, e.1–e.3.

- `[ ]` **Update release runbook with Cargo.lock sync + dry-run publish.**
  Add `cargo check` (to sync `Cargo.lock`) and
  `cargo publish --dry-run --features encryption` to AGENTS.md runbook step 3
  (after the version bump, before the commit). The v0.5.7 release failed
  because `Cargo.lock` wasn't committed alongside the version bump. The
  CHANGELOG [0.6.0] section claims `cargo publish --dry-run` was added, but
  it's only in the backfill instructions, not in the main runbook steps.
  Source: `docs/status/archived/2026-08-10_16-32_*` § d.1, e.1–e.3.

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
