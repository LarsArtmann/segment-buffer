# Session: performance benchmark expansion, compression-level sweep, default change

> **FULLY RESOLVED** — all work shipped. Forward-looking items harvested into
> `TODO_LIST.md` on 2026-08-10. Archived.

**Captured:** 2026-08-10 15:51
**Branch:** master (6 commits ahead of origin, 6 files uncommitted)
**Code state:** uncommitted working tree on top of `d5353aa` (perf: add zstd compression-level sweep across payload kinds)

---

## a) FULLY DONE

### 1. CI/local gate parity + LIMITATIONS.md + rustdoc (committed: `857d249`)

- **Clippy (fuzz)** step added to CI `test` job (was local-only via verify-gate.sh).
- **`RUSTDOCFLAGS=-D warnings`** added to CI doc build (local was stricter).
- **MSRV job** gained `components: clippy` + two clippy steps (default + encryption). Previously only `cargo check`.
- **`check-changelog-links.sh`** rewritten with `GITHUB_TOKEN` support (60/hr -> 5000/hr), HTTP 403 graceful degradation, DRY `check_tag()` helper. CI `changelog-links` job now passes `secrets.GITHUB_TOKEN`.
- **`verify-gate.sh`** rewritten with `--list` (prints all gate slugs) and `--only=X,Y,Z` (runs subset). 17 gates total (was 15; added `cargo-lock` and `msrv-consistency`). Slug validation with error on unknown names.
- **`docs/LIMITATIONS.md`** created (250 lines, 8 sections: process model, delivery semantics, durability, read consistency, data model, scope boundaries, format, operational). Each limitation has a "Why" rationale.
- **Crate-level rustdoc `# Limitations` section** added to `src/lib.rs` so it renders on docs.rs (where README and docs/ files are not served). Links to the full LIMITATIONS.md via GitHub URL.
- **README.md** callout blockquote linking to docs/LIMITATIONS.md.
- **AGENTS.md** updated: gate count 15->17, full gate list, docs/LIMITATIONS.md in project layout and living-docs enumeration.
- **TODO_LIST.md** all 4 CI/process items marked done with inline summaries.

### 2. Punctuation fix across all markdown (committed: `7a9db39`)

- All `,"` changed to `",` across 22 .md files (living + archived + perf + planning).
- JSON example in perf doc left as-is (correct JSON syntax).
- Private monitor365 link removed from README (committed: `59bc3c3`).

### 3. Benchmark baseline snapshot (committed: `ed88120`)

- All 10 benchmark targets' median times + throughput recorded in `docs/PERFORMANCE.md` as a "Baseline snapshot (2026-08-10, v0.5.6)" section.
- 39 data points across append, read, delete, recover, stats, durability, cipher, and segment_size_stats.
- Two missing bench entries (`bench_segment_size_stats`, `bench_cipher`) added to the Available benchmarks table.

### 4. Benchmark expansion (committed: `ecbb137`)

- **`bench_concurrent_append.rs`** (NEW): MPMC throughput benchmark. 1/2/4/8 threads, each appending 10k items. Measures both `append` (per-item lock) and `append_all` (per-batch lock). Registered in Cargo.toml.
- **`bench_append_all`** expanded: batch sizes 10/50/100/500/1k/5k/10k (was 100/1k/10k). Surfaces the crossover point where `append_all` beats loop `append`.
- **`bench_read_from` scan cache** expanded: 10/100/1k/**10k** segments (was 10/100/1k). The 10k data point shows whether the cache scales or breaks down.
- **`bench_cipher`** fixed: `Throughput::Elements(BATCH_SIZE)` added so the throughput column appears.

### 5. Scaling example upgrades (committed: `ecbb137` + uncommitted)

- **`--dir DIR` flag**: test on real disk instead of tmpfs. Cleans up after run unless `--keep` is passed.
- **`--encrypted` flag**: enables XChaCha20-Poly1305 encryption (requires `--features encryption`).
- **`--keep` flag**: don't delete `--dir` after the run.
- **Latency percentiles**: per-batch timing with p50/p95/p99 reported for both load and drain phases.
- **Flag-based arg parsing**: replaced positional-only parsing with `--flag` support while keeping positional args for backward compat.
- **Encryption/disk/encrypted status** printed in output header for reproducibility.

### 6. Compression-level sweep (committed: `d5353aa` + uncommitted data)

- **`scripts/compression-sweep.sh`** created: sweeps all 22 zstd levels x 4 payload kinds (uniform/text/json/random), outputs TSV.
- **Data captured**: 63 of 88 rows (uniform 1-22 complete, text 1-22 complete, json 1-19 complete). Missing: json 20-22, random 1-22. The `random` 1-5 data was captured separately via a quick inline run.
- **Conclusion documented**: level 1 is the sweet spot for a throughput buffer (2x faster than 3, negligible ratio loss). Full sweep TSV at `docs/perf/2026-08-10_compression-level-sweep.tsv`.

### 7. Default compression level changed 3 -> 1 (UNCOMMITTED)

- `src/lib.rs`: default `compression_level` field changed from 3 to 1. Doc comments updated.
- `benches/support.rs`: bench config changed from 3 to 1.
- `docs/PERFORMANCE.md`: section rewritten to document level 1 as default with sweep data citation.
- `docs/DOMAIN_LANGUAGE.md`: tradeoff table updated.
- Tests pass, clippy clean, docs build clean.

### 8. Concurrent append benchmark results (committed in `ecbb137`)

Key finding: `append_all` scales **better** under contention than per-item `append`:

| Threads | append (Melem/s) | append_all (Melem/s) |
|---------|-----------------|---------------------|
| 1       | 5.27             | 4.88                |
| 2       | 4.53             | 6.05                |
| 4       | 3.46             | 6.79                |
| 8       | 1.96             | 7.10                |

At 8 threads, `append_all` is **3.6x faster** than `append`. The mutex contention on per-item lock acquisition is the bottleneck; batching amortizes it.

### 9. Real-disk scaling test results

| Metric    | tmpfs       | real disk (/var/tmp) | encrypted (tmpfs) |
|-----------|------------|---------------------|-------------------|
| Load ips  | 379,663    | 404,893             | 300,471           |
| Drain ips | 1,278,091  | 1,420,703           | 1,157,150         |
| Load p99  | 16,785 us  | 16,427 us           | 21,722 us         |
| Drain p99 | 6,242 us   | 4,988 us            | 6,061 us          |

**Surprise:** real disk was *slightly faster* than tmpfs on this machine (NVMe with write-back cache vs tmpfs page-table overhead). The buffer is CPU-bound (zstd), not I/O-bound at this payload size.

---

## b) PARTIALLY DONE

### 1. Compression-level sweep data incomplete

- **63/88 rows captured** (71%). Missing: json 20-22, random 1-22 (random 1-5 captured separately inline, not in TSV).
- The sweep was killed because the user asked for status and the conclusion was already clear.
- The TSV file exists but is missing rows. It should either be completed or the missing rows filled in.

### 2. PERFORMANCE.md baseline snapshot is stale

- The baseline snapshot was recorded *before* the default compression level change (3->1) and before the benchmark expansion. The numbers reflect the old default (level 3).
- After committing the default change, the baseline should be re-run to reflect level 1.

### 3. Compression sweep not documented as a perf doc

- The TSV exists at `docs/perf/2026-08-10_compression-level-sweep.tsv` but there is no accompanying `.md` analysis file (unlike the 2026-07-21 scaling sweep which has a full writeup).

### 4. Uncommitted changes

- 6 files modified but not committed: the compression-level default change (src/lib.rs, docs/PERFORMANCE.md, docs/DOMAIN_LANGUAGE.md, benches/support.rs), plus the compression-sweep.sh rewrite and bench_concurrent_append formatting.
- The auto-git daemon may commit these at any time.

---

## c) NOT STARTED

1. ~~**CHANGELOG `[Unreleased]` entry** for all session changes (CI hardening, LIMITATIONS.md, benchmark expansion, compression sweep, default level change).~~ done — updated in v0.5.7 release
2. ~~**`cargo bench` re-run** after the default compression level change to update the PERFORMANCE.md baseline.~~ open — tracked in TODO_LIST.md
3. ~~**Memory usage benchmark** (peak RSS under load) — mentioned as a gap, not addressed.~~ aspirational, not tracked
4. ~~**Large payload (1KB-10KB/item) bench** at scale — mentioned as a gap, not addressed.~~ aspirational, not tracked

---

## d) TOTALLY FUCKED UP

### 1. The compression-sweep.sh awk parsing broke twice

- **First attempt**: the awk script didn't match the scaling example's output format at all. Fields were misaligned, produced garbage TSV. I didn't verify the parsing before launching the 88-run sweep.
- **Second attempt**: the `p99` field still parsed wrong because I used `$7` (word index) instead of a regex extraction from the `p99=VALUE` format. Again, didn't verify before launching.
- **Third attempt**: fixed with `sed 's/.*p99=//'`. Finally worked. But I burned ~10 minutes of the user's patience on two failed sweeps before getting it right.
- **Root cause**: I should have done a single-run dry test, verified the TSV output, THEN launched the full sweep. Classic "test before you scale" failure.

### 2. The scaling example doc comment had `>` instead of `//!` on three lines

- The multiedit that rewrote the doc comment produced `>#` instead of `//! #` on three lines. This was a botched find-replace that I didn't catch until the compiler rejected it. Should have built immediately after the edit.

### 3. The `_tmp_keep` variable warning

- After adding `--dir` support, the tempdir keep logic left an unused-mut warning. Fixed with `#[allow(unused_mut)]`, which is correct but ugly. A cleaner approach would have been to restructure the dir-handling logic from the start.

### 4. Messy payload_kind/payload_mult arg parsing

- The first version of the flag parser had duplicate/conflicting parsing of positions 4/5 (payload_mult vs payload_kind). I wrote a convoluted fallback that tried to detect whether position 4 was a number or a kind string. Then I deleted it and used the simple positional approach (4=payload_mult, 5=payload_kind). Should have been simple from the start.

### 5. CI has NOT been verified green after ci.yml changes

- The CI hardening changes (clippy-fuzz, MSRV clippy, RUSTDOCFLAGS, changelog-links GITHUB_TOKEN) were committed in `857d249` but **nobody has run `gh run list` since**. The changes pass locally but are unverified on CI runners (especially macOS).

---

## e) WHAT WE SHOULD IMPROVE

### Process

1. **Always dry-test scripts before launching long sweeps.** Run one iteration, verify the output, THEN scale. I burned 3 sweep attempts on parsing bugs that a single dry run would have caught.
2. **Always build immediately after large edits.** The `//!` -> `>` doc comment bug would have been caught instantly.
3. **Commit after each logical change, not at the end.** The default compression level change is sitting uncommitted alongside sweep script fixes and benchmark formatting changes. These should be separate commits.
4. **Run `gh run list` after pushing CI changes.** The CI hardening commit is unverified on actual runners.

### Benchmark coverage

5. **The micro-benches still use uniform-equivalent payloads.** The scaling example proved that realistic payloads (text/json) are 14x slower, but the criterion benches don't reflect this. A `bench_append_realistic` variant with text payloads would make the perf tables honest.
6. **No memory-usage measurement.** Peak RSS under load is a complete blank spot.
7. **The concurrent-append bench uses a Barrier but doesn't pin threads.** On a 32-core machine, the OS scheduler can place threads on the same core, artificially inflating contention numbers.

### Documentation

8. **The compression sweep needs a proper `.md` analysis doc** alongside the TSV, like the 2026-07-21 scaling sweep has.
9. **PERFORMANCE.md baseline is stale** — it reflects level 3, the old default.
10. **LIMITATIONS.md is not in the lychee link-check** — the new doc has internal links that haven't been verified.

---

## f) Up to 50 things to do next

> **Harvested (2026-08-10).** Actionable items extracted into `TODO_LIST.md`.
> Remaining items are aspirational brainstorm, not tracked work.

### High priority (blocking/uncommitted)

1. **Commit the uncommitted compression-level default change** (src/lib.rs, docs, benches).
2. **Add CHANGELOG `[Unreleased]` entry** for all session changes.
3. **Verify CI is green** after the ci.yml changes: `gh run list --limit 4`.
4. **Push** and verify CI stays green on remote runners.
5. **Complete the compression sweep** (fill in json 20-22 and random 1-22 in the TSV).

### Benchmark improvements

6. **Re-run all benchmarks with new default (level 1)** and update PERFORMANCE.md baseline.
7. **Add realistic-payload variants to micro-benchmarks** (text/json, not just uniform-equivalent).
8. **Write `docs/perf/2026-08-10_compression-level-sweep.md`** analysis doc with charts/conclusions from the TSV.
9. **Add `Throughput::Elements` to `bench_segment_size_stats`** (it measures a scan, not elements, but a "segments scanned/sec" throughput would be informative).
10. **Add thread-pinning to `bench_concurrent_append`** using `core_affinity` crate for stable contention numbers.
11. **Add a `bench_read_from_large_segments` variant** that reads 100k+ items from a single large segment (vs many small segments).
12. **Add memory-usage tracking** to the scaling example (peak RSS via `/proc/self/status` VmHWM on Linux).
13. **Add a concurrent-read benchmark** (multiple reader threads calling `read_from` simultaneously).
14. **Add a mixed read/write benchmark** (producer + consumer running concurrently, the actual cloud-sync workload).
15. **Benchmark `delete_acked` at 100k segments** (the bench tops out at 10k; long-running buffers may have more).
16. **Add a `bench_iter_from` target** (owned-item iterator is benchmarked only via `bench_read_vs_for_each`).
17. **Run scaling test on actual spinning disk** (not NVMe) to measure the I/O-bound regime.
18. **Run scaling test with `Maximal` durability** on real disk to measure the fsync-bound regime.
19. **Run scaling test with AES-256-GCM** (vs XChaCha20) at scale to measure AES-NI advantage.
20. **Add a long-running stability test** (1B items, verifying throughput doesn't degrade over hours).

### Code improvements

21. **Consider making compression optional** (feature flag `compression` that disables zstd entirely for raw CBOR segments). Some users may want this for incompressible data.
22. **Consider compression algorithm negotiation** (lz4 for speed, zstd for ratio, none for incompressible). This is envelope v2 territory but worth scoping.
23. **The `LatencyHistogram` in scaling.rs allocates a `Vec<f64>` and clones+sorts on every `percentile()` call.** For a production harness, use a real histogram (HDR histogram, t-digest).
24. **The scaling example's flag parser is hand-rolled.** Consider `clap` for proper `--help`, validation, and error messages.
25. **`bench_concurrent_append` creates items inside the timed region** for the `append` variant (each call to `buf.append(Item { ... })` allocates a String). Move item construction outside the barrier.

### Documentation

26. **Update README "Crash behavior" section** — it still says "default remains `Segment`" but that's overdue for a flip to `Throughput` (per AGENTS.md policy).
27. **Update README compression mention** — the docs say "zstd level 3" in several places; update to level 1.
28. **Add a "Performance" section to the README** summarizing the key numbers (load throughput, drain throughput, compression-level recommendation).
29. **Document the concurrent-append finding** in PERFORMANCE.md (append_all 3.6x faster at 8 threads).
30. **Document the real-disk finding** in PERFORMANCE.md (NVMe ~= tmpfs; buffer is CPU-bound).
31. **Add the compression sweep to the `docs/perf/` index** in PERFORMANCE.md's "Controlled baselines" section.
32. **Run lychee on docs/LIMITATIONS.md** to verify internal links.
33. **Update FEATURES.md** if any feature status changed.
34. **Update AGENTS.md** with the new bench target (`bench_concurrent_append`) and the compression-level default change.
35. **Add a `docs/perf/2026-08-10_concurrent-append-and-real-disk.md`** snapshot for the concurrent + real-disk findings.

### CI/process

36. **Run full `scripts/verify-gate.sh` end-to-end** (including loom, lychee, supply-chain, nix flake check) — has not been run this session.
37. **Audit `nix.yml` and `fuzz.yml`** for parity with the local gate (the parity audit covered `ci.yml` only).
38. **Consider adding `bench_concurrent_append` to CI** as a non-blocking perf-regression check.
39. **Consider adding the compression sweep to a weekly CI job** (like supply-chain-report).
40. **Tag a release** if the changes are stable (v0.5.7 with default compression level 1).

### Testing

41. **Add property tests for the compression-level default** — verify that segments written with level 1 decode correctly at any level (they should, since level is encode-only).
42. **Add a test that `SegmentConfig::default().compression_level == 1`** — catches accidental default drift.
43. **Test the scaling example's `--encrypted` flag end-to-end** (it was compiled but the encrypted scaling run used the flag successfully, so this is partially done).
44. **Add a test for the scaling example's flag parsing** (`--dir`, `--encrypted`, `--keep`, unknown flags).

### Cleanup

45. **Remove the `compression_level: 3` from test configs** in `src/tests.rs` (8 occurrences) — they're explicit overrides that now differ from the default, which is fine but inconsistent. Consider whether they should be 1 (for speed) or left at 3 (for historical comparability).
46. **Remove the `compression_level: 3` from example configs** (10 occurrences across examples/) — same consideration.
47. **Consolidate the `scripts/compression-sweep.sh` awk/sed parsing** into a single robust parser.
48. **Add the `scripts/compression-sweep.sh` to the verify-gate** as an optional perf gate.
49. **Clean up the `bench_concurrent_append.rs` item construction** — items are built inside the timed closure for the `append` variant.
50. **Consider renaming `bench_read_vs_for_each` to `bench_read_vs_iter`** to match the actual API names (`read_from` vs `iter_from`/`for_each_from`).

---

## g) Questions (3)

### ~~Q1: Should we ship a v0.5.7 release with the compression-level default change?~~

> **Resolved.** Shipped as v0.5.7 with level-1 default.

Changing the default from 3 to 1 is a **behavioral change** for every existing user. Their segments will compress slightly less (3.1x -> 3.2x ratio is actually *better* at level 1 for text, but disk footprint for uniform payloads increases: 4.4 MiB -> 4.4 MiB at 1M, no change). Existing segments at any level still decode correctly (the level is encode-only, stored in the zstd frame header). But users who explicitly want level 3 must now set it. **I cannot answer this** because it depends on your release cadence preference and whether monitor365 or other consumers depend on the current default.

### ~~Q2: Should the compression sweep be completed (json 20-22 + all random levels)?~~

> **Resolved.** Sweep declared done at 63/88 rows — conclusion is clear.

The conclusion is already clear from 63/88 rows. The missing rows are the slowest (json 20-22 take ~5 min each) and the most predictable (random is near-incompressible at all levels). Completing them adds completeness but no new insight. **I cannot answer this** because it's a completeness-vs-time tradeoff that depends on how much you value a full matrix vs a representative sample.

### ~~Q3: Should the micro-benchmarks (criterion) be updated to use realistic payloads (text/json) instead of uniform-equivalent?~~

> **Open — aspirational.** Measurement-fidelity tradeoff, not tracked.

The scaling example proved uniform overstates throughput by ~14x. The criterion benches all use `format!("payload-{n}")` which compresses extremely well (similar to uniform). Making them realistic would produce lower but more honest numbers in the PERFORMANCE.md tables. But it would also make regression detection harder (more variance from zstd). **I cannot answer this** because it's a measurement-fidelity vs regression-detection-sensitivity tradeoff.
