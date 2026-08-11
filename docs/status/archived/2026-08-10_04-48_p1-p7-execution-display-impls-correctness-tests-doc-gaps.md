# Status Report: P1-P7 Execution — Display Impls, Correctness Tests, Doc Gaps, Code Wins

> **FULLY RESOLVED** — all work shipped. Forward-looking items harvested into
> `TODO_LIST.md` on 2026-08-10. Archived.

**Date:** 2026-08-10 04:48 CEST
**Author:** Crush session (continuation of docs-health → TODO_LIST execution)
**Commits:** `109f107`, `c86c347`, `ec6f16c`
**Verification:** 15/15 gates green. CI: success (6m10s). Nix: success (3m8s).
**Working tree:** Clean. `origin/master` up to date.

---

## What this session did

Executed a 7-phase Pareto plan (`docs/planning/archived/2026-08-10_03-59_*.md`)
that shipped 11 TODO_LIST items: 3 API-ergonomics improvements, 3 Display trait
impls, 8 property tests, 1 helper extraction, 1 cipher parity test, and 4 doc-gap
fixes. Every change is purely additive (no behavior change, no hot-path
modification, no on-disk format change, no new dependency, no API signature
change). The anti-Verschlimmbesserung checklist was followed for each phase.

---

## a) FULLY DONE (verified green this session)

### P1 — Session cleanup

- **AGENTS.md archive convention documented.** The "Documentation health
  cadence" section now mentions `docs/planning/archived/` alongside the
  existing `docs/status/archived/` reference. Previously only the latter was
  documented.
- **P1.1-P1.3** (archiving 02:37/02:42 docs, CHANGELOG entries) were committed
  by the auto-git daemon as `2480057`.

### P2 — Display impls (3 types)

- **`DurabilityPolicy`** — `Display` produces stable lowercase variant names:
  `"maximal"`, `"segment"`, `"throughput"`. Follows the existing `FlushPolicy`
  Display pattern.
- **`BufferStats`** — `Display` produces a single-line summary with all 8
  fields in fixed order: `pending`, `seqs=head..latest (head=X next=Y)`,
  `disk=NbB/MaxB in N segments`, `pressure=X.XX`.
- **`SegmentConfig`** — `Display` produces a summary that masks the cipher as
  `[set]`/`[none]`, preventing key material leakage into logs. Matches the
  existing `Debug` masking behavior.
- **4 unit tests** added: `durability_policy_display_formats_each_variant`,
  `buffer_stats_display_formats_all_fields`,
  `segment_config_display_masks_cipher` (default features),
  `segment_config_display_masks_cipher_when_set` (encryption feature, uses
  XChaCha20Poly1305Cipher).

### P3 — Trivial code wins

- **4 `p-{i}` → `prop_item(u64::from(i))`** conversions in
  `src/property_tests.rs`. Verified zero tests assert on payload string content
  (only on ids, seqs, counts, and ordering).
- **`#[doc(alias = "backlog")]`** on `pending_count()` — rustdoc search
  discoverability.
- **`#[must_use]`** on `BufferStats` struct — compile-time lint for discarded
  stats snapshots.

### P4 — Doc gaps

- **Crate-level `# Guarantees` section** added to `src/lib.rs` rustdoc.
  Documents: panic-free public API (Clippy-enforced), single-process `flock`
  enforcement, filename-based crash recovery. Previously implicit in CI but
  invisible to API consumers reading docs.rs.
- **Examples table** in `src/lib.rs`: added `batch_or_interval_min` and
  `segment_tuning` rows (2 of 14 examples were missing).
- **FEATURES.md examples inventory**: expanded from 3 to all 14 examples.

### P5 — Correctness property tests (8 new)

- **`compute_store_pressure` (4 properties):** zero-max returns 0.0, always in
  `[0.0, 1.0]`, monotone non-decreasing in bytes, saturates at exactly 1.0
  when bytes exceed max.
- **`percentile_of_sorted` (4 properties):** empty slice returns 0, all-equal
  slice returns that value for all pct, result is always an actual element
  (never interpolated), p0 returns min and p100 returns max.

### P6 — seq_to_index helper

- Extracted `fn seq_to_index(seq: u64, base: u64) -> usize` private associated
  function. Replaced 3 duplicated
  `usize::try_from(seq.saturating_sub(base)).unwrap_or(usize::MAX)` call sites
  in `read_from` and `for_each_from` (both the on-disk and in-memory phases).

### P7 — XChaCha20 encrypted test

- `segment_size_stats_works_with_xchacha20_encrypted_segments` — full parity
  test mirroring the existing AES-GCM test. Cross-checks every field against a
  brute-force directory scan.

### Post-execution doc closure

- **CHANGELOG `[Unreleased]`** updated with all additions, changes, and
  documentation entries.
- **TODO_LIST** pruned: 11 completed items removed, empty "API ergonomics"
  section cleaned up. 13 items remain (all deferred items D1-D15 subset).
- **Planning doc** marked `FULLY EXECUTED`.

---

## b) PARTIALLY DONE

Nothing is in a partial state. All 7 phases are either fully done or not
started (deferred by design).

---

## c) NOT STARTED (deferred by design — 15 items)

These were explicitly deferred in the Pareto plan with documented rationale.
They are tracked in `TODO_LIST.md`.

| ID  | Task | Why deferred |
|-----|------|-------------|
| D1  | ~~Flip DurabilityPolicy default Segment→Throughput~~ done — shipped in v0.6.0 | Changes default behavior — needs release scope |
| D2  | ~~Loom coverage for for_each_from~~ done — shipped in v0.5.6 | Gate already takes 219s; plan first |
| D3  | ~~Loom test for iter_from~~ done — shipped in v0.5.6 | Same as D2 |
| D4  | ~~FlushPolicy::validate() method~~ done — shipped in v0.5.6 | Design question (Result vs panic) |
| D5  | ~~Seal SegmentStore trait~~ done — shipped in v0.5.6 | Semver implications under loom feature |
| D6  | ~~PartialEq for SegmentConfig~~ done — shipped in v0.5.6 | Blocked by Arc<dyn SegmentCipher> |
| D7  | ~~Fuzz target for for_each_from~~ done — shipped in v0.5.6 (not in CI fuzz matrix — tracked in TODO_LIST) | High effort, statistical coverage exists |
| D8  | ~~bench_segment_size_stats~~ done — shipped in v0.5.6 (never run — tracked in TODO_LIST) | High effort, lower immediate value |
| D9  | ~~bench_cipher~~ done — shipped in v0.5.6 (never run — tracked in TODO_LIST) | High effort, lower immediate value |
| D10 | ~~Property test: publish_disk_stats~~ done — shipped in v0.5.6 | Statistical coverage exists |
| D11 | ~~Property test: delete_acked idempotency~~ done — shipped in v0.5.6 | Loom proof exists |
| D12 | ~~Stress test: segment_size_stats concurrent~~ done — shipped in v0.5.6 | Statistical coverage exists |
| D13 | ~~CI parity audit + clippy on MSRV~~ done — shipped in v0.5.7 | Don't destabilize green CI |
| D14 | ~~check-changelog-links.sh robustness~~ done — shipped in v0.5.7 | Don't destabilize green CI |
| D15 | ~~verify-gate.sh --list/--only~~ done — shipped in v0.5.7 | Don't destabilize green CI |

---

## d) TOTALLY FUCKED UP

Nothing is fucked up at the code level — all 15 gates pass, CI is green,
tests are green. But there were **process failures** worth documenting:

### Process failure 1: Auto-git daemon committed twice before I could

The context explicitly warned: "MUST commit BEFORE running the gate, then
amend if the gate finds issues." I did NOT commit after completing P1-P3 or
before running the verification gate. The daemon committed `109f107` (P1+P3)
and `c86c347` (P2+P4-P7) before I could. The daemon's commit messages were
actually good (detailed, accurate), so no harm done — but I ceded control of
the commit boundary, which means I could not have amended if the gate had
found issues. **Next time: commit immediately after each phase group.**

### Process failure 2: BufferStats Display test assertion mismatch

My first test run failed because I wrote `s.contains("segments=4")` but the
Display format produces `"in 4 segments"`. I had to iterate. This happened
because I wrote the Display impl and the test in separate steps without
running the test immediately. **Should have tested each Display impl
immediately after writing it.**

### Process failure 3: Clippy `field_reassign_with_default` hit

The `segment_config_display_masks_cipher_when_set` test mutated
`config.cipher` after `SegmentConfig::default()`. Under the project's strict
clippy (pedantic + nursery + restrictions), this is a lint failure. I should
have used struct update syntax (`..SegmentConfig::default()`) from the start.
**Should have known better — this project has the strictest clippy config
I've worked with.**

### Process failure 4: `edit` tool failed twice on non-unique matches

The `assert!(s.p90_bytes <= s.max_bytes)` pattern appeared twice (the existing
AES-GCM test and my new XChaCha20 test). I had to use more context to
disambiguate. **Should have read the surrounding lines more carefully before
attempting the edit.**

### Process failure 5: CHANGELOG entries added after code commits

The code changes (Display impls, property tests, etc.) were committed without
CHANGELOG entries. The CHANGELOG was updated in a follow-up docs commit
(`ec6f16c`). Ideally, the CHANGELOG entry should accompany the code change in
the same commit. **The auto-git daemon's commit boundary beat me to it.**

---

## e) WHAT WE SHOULD IMPROVE

### Immediate (this codebase)

1. **`#[inline]` on `seq_to_index`.** It's called in the read hot path
   (`read_from`, `for_each_from`). The compiler likely inlines it anyway, but
   the annotation is free and documents intent.

2. **BufferStats Display: human-readable bytes.** `disk=4096B/1048576B` is
   hard to read in logs. `disk=4KB/1MB` or `disk=4.0KB/1.0MB` would be more
   operator-friendly. This is a design question, not a bug.

3. **Display for `RecoveryReport` and `SegmentSizeStats`.** These are two more
   Debug-only types that users might want to log. The pattern is established
   now; adding them is low-effort.

4. **`# Guarantees` section placement.** I placed it between
   `# Delivery guarantees` and `# Schema evolution of T`. The two guarantee
   sections (`# Delivery guarantees` and `# Guarantees`) are close together
   and might benefit from consolidation or clearer naming (e.g.,
   `# Operational guarantees` for the new one).

5. **AGENTS.md not updated with new abstractions.** The `seq_to_index` helper
   and the three new Display impls are not mentioned in AGENTS.md. The
   `seq_to_index` is a new named abstraction in the read path — it should be
   documented alongside the `compute_store_pressure` / `publish_disk_stats`
   mention.

6. **Property test for `seq_to_index`.** The new helper has no dedicated test.
   It's trivially correct (delegates to `saturating_sub` + `try_from`), but a
   quick property test (seq < base → 0, seq == base → 0, seq > base →
   seq - base) would document the contract.

### Process

7. **Commit before the gate, always.** The daemon beat me twice. Next session,
   commit after each completed phase group, not after all phases.

8. **Test each impl immediately.** Don't batch — write impl, write test, run
   test, move on. Saves iteration round-trips.

9. **Read clippy lint rules before writing test code.** This project denies
   `field_reassign_with_default`. Struct-update syntax should be the default
   in all test code here.

---

## f) Up to 50 things to get done next

> **Harvested (2026-08-10).** Actionable items extracted into `TODO_LIST.md`.
> Remaining items are aspirational brainstorm, not tracked work.

### Release-scoped (high impact)

1. **Flip DurabilityPolicy default** Segment → Throughput with deprecation note
2. **Plan and ship v0.6.0** (or v0.5.6 if additive-only) — the unreleased
   changes are non-breaking and could ship
3. **Write v0.6.0 CHANGELOG** section, move `[Unreleased]` entries under it
4. **Soak test the Display impls** in a real logging scenario (log format
   stability, grep-ability)

### Correctness (high value)

5. **Loom coverage for `for_each_from`** snapshot-then-release-lock pattern
6. **Loom test for `iter_from`** materialising iterator path
7. **Property test for `publish_disk_stats`** atomic counter correctness
8. **Property test for `delete_acked` idempotency** under concurrent append
9. **Stress test for `segment_size_stats`** under concurrent flush + delete
10. **Fuzz target for `for_each_from`** with arbitrary start/limit
11. **Property test for `seq_to_index`** — boundary cases (seq==base, overflow)
12. **Property test for `append_all`** sequence contiguity at scale (1M+ items)

### API ergonomics

13. **Display for `RecoveryReport`** — another Debug-only type users log
14. **Display for `SegmentSizeStats`** — tuning output is logged in examples
15. **`#[inline]` on `seq_to_index`** — read hot path
16. **BufferStats Display: human-readable byte formatting** (4KB not 4096B)
17. **`FlushPolicy::validate()` method** — move debug_asserts into reusable fn
18. **Seal the `SegmentStore` trait** — enforce "not semver" claim
19. **`PartialEq` for `SegmentConfig`** — needs design for cipher field
20. **Consider `serde::Serialize` for `BufferStats`** — structured logging
21. **Consider `AsRef<str>` or `Into<String>` for Display types** — log macros
22. **Display format stability test** — parse and round-trip (regression guard)

### Documentation

23. **AGENTS.md: document `seq_to_index`** alongside `compute_store_pressure`
24. **AGENTS.md: document the three new Display impls**
25. **docs/DOMAIN_LANGUAGE.md: add Display format stability** to the glossary
26. **Consolidate `# Delivery guarantees` and `# Guarantees`** in crate rustdoc
27. **Visually verify README rendering** on GitHub, docs.rs, mobile viewport
28. **Add `docs/status/archived/README.md`** explaining the archive convention
29. **Add `docs/planning/archived/README.md`** same
30. **Review FEATURES.md table column widths** — 14 rows may need adjustment

### Performance

31. **`bench_segment_size_stats`** — quantify O(n_segments) scan cost
32. **`bench_cipher`** — AES-GCM / XChaCha20 overhead vs no-cipher baseline
33. **Nightly benchmark CI** — track perf regressions over time
34. **Benchmark Display impls** — ensure they don't allocate on the hot path
35. **jscpd/dedup analysis in CI** — automated duplication detection

### CI / process

36. **CI vs local gate parity audit** — enumerate and diff every check
37. **Clippy with full lint stack on MSRV CI job** — currently only cargo check
38. **`check-changelog-links.sh` robustness** — rate-limit detection, GITHUB_TOKEN
39. **`verify-gate.sh --list` and `--only=`** — faster local iteration
40. **Optimize loom gate runtime** — 219s is painful; profile and reduce
41. **Add supply-chain publisher provenance** to the gate (informational)

### Long-term (ROADMAP)

42. **Envelope v2** — Blake3 checksum, compression negotiation, cipher-type marker
43. **Streaming AEAD cipher** (RFC 8450 chunked format) — bound memory on large segments
44. **Second `SegmentStore` impl** — e.g., S3-backed or in-memory for testing
45. **Async I/O exploration** — tokio integration for the drain loop
46. **Cloud-sync extraction** — pull monitor365's sync loop into its own crate
47. **Cursor file reconsideration** — re-evaluate the rejected cursor-file proposal

### Code quality

48. **Review whether `# Guarantees` duplicates `# Delivery guarantees`** content
49. **Add doc cross-references** between Display impls and their types
50. **Consider `impl From<BufferStats> for String`** for ergonomic logging

---

## g) Questions I cannot figure out myself

### ~~1. Should the next release be v0.5.6 or v0.6.0?~~ done — resolved: v0.5.6 and v0.6.0 both shipped

The `[Unreleased]` changes are purely additive (Display impls, `#[must_use]`,
`#[doc(alias)]`, property tests, helper extraction, doc sections). Semver says
additive = patch (0.5.6). But the DurabilityPolicy default flip (D1) — the
highest-impact remaining TODO — is a behavior change that warrants a minor
bump (0.6.0). **Should I ship 0.5.6 now with just the additive changes, or
bundle the DurabilityPolicy flip into 0.6.0?**

### ~~2. Should BufferStats Display format bytes as human-readable?~~ done — resolved: user chose human-readable (4.0KB); shipped in v0.5.6

`disk=4096B/1048576B` is machine-parseable but hard to read.
`disk=4.0KB/1.0MB` is human-friendly but harder to parse and locale-sensitive.
The Display format is documented as "stable across releases so operators can
parse it in log-scraping tools." **Do you want raw bytes (current) or
human-readable units?**

### ~~3. What should I work on next — the DurabilityPolicy flip, or the loom coverage gap?~~ done — resolved: both shipped (loom in v0.5.6, DurabilityPolicy flip in v0.6.0)

Both are high-value. The DurabilityPolicy flip is the single highest-impact
TODO item (changes default behavior for the target use case). The loom
coverage for `for_each_from` is the single highest-risk correctness gap (the
panic-free refactor introduced a new concurrency surface that is only
statistically proven, not exhaustively). **Which do you want first: the
behavior change (ship impact) or the proof (safety)?**
