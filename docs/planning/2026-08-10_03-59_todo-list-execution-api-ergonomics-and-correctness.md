# Pareto Plan: TODO_LIST Execution — API Ergonomics + Doc Gaps + Correctness

**Date:** 2026-08-10 03:59 CEST
**Author:** Docs-health session (continuation)
**Status:** FULLY EXECUTED — all 7 phases (P1–P7) shipped, 15/15 gates green, CI+Nix green. Commits `109f107`, `c86c347`.

---

## Context

The TODO_LIST was rebuilt this session from 7 → 24 items across 7 categories.
All items were verified against code. This plan selects the subset that
delivers maximum value with zero Verschlimmbesserung risk, and executes it.

**Execution principle:** Only touch code where the change is zero-risk
(additive, no behavior change, no hot-path modification). Items that change
published behavior (DurabilityPolicy flip), carry semver risk (seal trait),
are blocked (PartialEq), or are high-effort/low-immediate-value (loom, fuzz,
benchmarks, CI changes) are PLANNED but NOT EXECUTED this session.

---

## Pareto Breakdown

### The 1% that delivers 51%

**Session cleanup + trivial code wins.** Close the docs-health loop (archive
own session docs, update CHANGELOG/AGENTS.md) and ship 3 zero-risk code
improvements that are ~2 min each:
- Convert 4 `"p-{i}"` to `prop_item(i)` — DRY, zero behavior change
- `#[doc(alias = "backlog")]` on `pending_count()` — discoverability
- `#[must_use]` on `BufferStats` struct — lint correctness

### The 4% that delivers 64%

**Above + Display impls.** Three types are `Debug`-only today while
`FlushPolicy` already ships `Display`. Adding `Display` to
`DurabilityPolicy`, `BufferStats`, and `SegmentConfig` gives users
human-readable output for logging and diagnostics — the exact use case the
TODO_LIST cites. Zero risk (additive trait impl, no behavior change).

### The 20% that delivers 80%

**Above + doc gaps + correctness tests.**
- Examples table in `src/lib.rs` is missing 2 of 14 examples (visible doc gap)
- Crate-level rustdoc has no `# Guarantees` section (README does)
- `compute_store_pressure` is a pure function with no dedicated property test
- `percentile_of_sorted` edge cases (n=0, duplicates) unproven by property test

### The remaining 20% (to reach 100%)

**Planned but deferred** — high effort, behavior change, or blocked. See the
"Deferred" table below for rationale.

---

## Phase 1: Comprehensive Plan (30–100 min tasks)

Sorted by impact × customer-value, then effort within tier.

| ID  | Task                                                                                  | Impact      | Effort  | Customer Value | Execute? | Category     |
| --- | ------------------------------------------------------------------------------------- | ----------- | ------- | -------------- | -------- | ------------ |
| P1  | **Session cleanup:** archive own docs, CHANGELOG, AGENTS.md update                    | 🔴 Critical | 20min   | Internal       | ✅ YES   | Process      |
| P2  | **Display impls** for DurabilityPolicy, BufferStats, SegmentConfig                    | 🟠 High     | 35min   | User-facing    | ✅ YES   | API          |
| P3  | **Trivial code wins:** p-{i}→prop_item, doc(alias), must_use                          | 🟡 Medium   | 10min   | Code quality   | ✅ YES   | Code         |
| P4  | **Doc gaps:** Examples table + Guarantees section + FEATURES inventory                | 🟠 High     | 25min   | User-facing    | ✅ YES   | Docs         |
| P5  | **Correctness tests:** compute_store_pressure + percentile edge cases                 | 🟡 Medium   | 30min   | Correctness    | ✅ YES   | Testing      |
| P6  | **seq_to_index helper extraction**                                                    | 🟢 Low      | 15min   | Code quality   | ✅ YES   | Code         |
| P7  | **XChaCha20 encrypted segment_size_stats test**                                       | 🟢 Low      | 12min   | Test parity    | ✅ YES   | Testing      |
| D1  | **Flip DurabilityPolicy default** Segment → Throughput                                | 🔴 Critical | 40min   | User-facing    | ❌ DEFER  | Release      |
| D2  | **Loom coverage for for_each_from** snapshot-then-release-lock                        | 🟠 High     | 60min   | Correctness    | ❌ DEFER  | Testing      |
| D3  | **Loom test for iter_from**                                                           | 🟡 Medium   | 45min   | Correctness    | ❌ DEFER  | Testing      |
| D4  | **FlushPolicy::validate() method**                                                    | 🟡 Medium   | 25min   | Code quality   | ❌ DEFER  | Code         |
| D5  | **Seal SegmentStore trait**                                                           | 🟡 Medium   | 30min   | API safety     | ❌ DEFER  | Code         |
| D6  | **Derive PartialEq for SegmentConfig**                                                | 🟢 Low      | 20min   | Test ergonomics | ❌ BLOCKED | Code        |
| D7  | **Fuzz target for for_each_from**                                                     | 🟡 Medium   | 60min   | Correctness    | ❌ DEFER  | Testing      |
| D8  | **bench_segment_size_stats**                                                          | 🟢 Low      | 40min   | Performance    | ❌ DEFER  | Benchmark    |
| D9  | **bench_cipher**                                                                      | 🟢 Low      | 40min   | Performance    | ❌ DEFER  | Benchmark    |
| D10 | **Property test: publish_disk_stats correctness**                                     | 🟡 Medium   | 30min   | Correctness    | ❌ DEFER  | Testing      |
| D11 | **Property test: delete_acked idempotency** under concurrent append                   | 🟡 Medium   | 30min   | Correctness    | ❌ DEFER  | Testing      |
| D12 | **Stress test: segment_size_stats under concurrent flush+delete**                     | 🟢 Low      | 25min   | Correctness    | ❌ DEFER  | Testing      |
| D13 | **CI parity audit + clippy on MSRV job + verify-gate.sh improvements**                | 🟡 Medium   | 60min   | Process        | ❌ DEFER  | CI           |
| D14 | **check-changelog-links.sh robustness**                                               | 🟢 Low      | 20min   | Process        | ❌ DEFER  | CI           |
| D15 | **DurabilityPolicy default flip** (same as D1, listed for emphasis)                   | 🔴 Critical | 40min   | User-facing    | ❌ RELEASE | Release     |

### Deferred rationale

| ID  | Why deferred                                                                          |
| --- | ------------------------------------------------------------------------------------- |
| D1  | Changes default behavior — needs release scope, CHANGELOG, deprecation note. Not a drive-by fix. |
| D2  | Loom gate already takes 219s. Adding more loom tests without optimizing the runtime makes the gate worse, not better. Plan first, add when the gate can absorb the cost. |
| D3  | Same as D2. Statistical coverage exists (`iter_from_invariant_under_concurrent_flush_and_delete`). |
| D4  | Moving `debug_assert!` into a method changes the call pattern. Needs careful design: should it be `fn validate(&self)` returning `Result` or panicking in debug? Design question, not a drive-by fix. |
| D5  | Sealing changes the public API surface under the `loom` feature. Semver implications. Needs a design decision about whether `loom` is truly "not semver." |
| D6  | `Arc<dyn SegmentCipher + Send + Sync>` blocks `PartialEq` derive. Would need a custom impl that compares cipher presence (not content), which is a design question. |
| D7-D12 | High effort, lower immediate value. Statistical/stress coverage exists for all of these. Add when the specific gap becomes painful. |
| D13-D14 | CI changes risk breaking CI. The gate is green now; don't destabilize it for process polish. |

---

## Phase 2: Micro-Task Breakdown (max 12 min each)

### P1 — Session cleanup (20min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P1.1  | Archive 02:37 status report: add resolution header, `git mv` to archived/         | 4min   | —          |
| P1.2  | Archive 02:42 Pareto plan: add FULLY EXECUTED header, `git mv` to archived/       | 4min   | —          |
| P1.3  | Add CHANGELOG `[Unreleased]` Documentation sub-entry for this session's work      | 4min   | —          |
| P1.4  | Update AGENTS.md "Documentation health cadence" to mention `docs/planning/archived/` | 4min   | —          |
| P1.5  | Update any internal links pointing to the now-archived 02:37/02:42 docs           | 4min   | P1.1, P1.2 |

### P2 — Display impls (35min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P2.1  | Implement `Display` for `DurabilityPolicy` (3 variants, follow FlushPolicy pattern) | 8min   | —          |
| P2.2  | Implement `Display` for `BufferStats` (format key fields: pending, segments, pressure) | 12min  | —          |
| P2.3  | Implement `Display` for `SegmentConfig` (mask cipher like Debug does)            | 10min  | —          |
| P2.4  | Add unit tests for all three Display impls                                        | 5min   | P2.1-P2.3  |

### P3 — Trivial code wins (10min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P3.1  | Convert 4 `"p-{i}"` PropItem constructions at lines 297/323/348/376 to `prop_item(i)` | 4min   | —          |
| P3.2  | Add `#[doc(alias = "backlog")]` on `pending_count()`                              | 2min   | —          |
| P3.3  | Add `#[must_use]` to `BufferStats` struct                                         | 2min   | —          |
| P3.4  | Verify: `cargo fmt`, `cargo clippy`, `cargo test` still clean                     | 2min   | P3.1-P3.3  |

### P4 — Doc gaps (25min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P4.1  | Add `batch_or_interval_min` and `segment_tuning` rows to Examples table in `src/lib.rs` | 5min   | —          |
| P4.2  | Add `# Guarantees` section to crate-level rustdoc (adapt from README)             | 10min  | —          |
| P4.3  | Expand FEATURES.md examples inventory (list all 14 or link to directory)          | 10min  | —          |

### P5 — Correctness tests (30min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P5.1  | Write property test for `compute_store_pressure`: max==0→0.0, clamp at 1.0, monotonic in bytes | 12min  | —          |
| P5.2  | Write percentile edge-case property test: duplicate values (ties)                 | 10min  | —          |
| P5.3  | Write percentile edge-case property test: n=0 (empty slice)                       | 8min   | —          |

### P6 — seq_to_index helper (15min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P6.1  | Add `fn seq_to_index(seq, base) -> usize` helper to SegmentBuffer impl            | 5min   | —          |
| P6.2  | Replace 3 call sites in read_from/for_each_from                                   | 5min   | P6.1       |
| P6.3  | Verify: `cargo fmt`, `cargo clippy`, `cargo test`                                 | 5min   | P6.2       |

### P7 — XChaCha20 encrypted test (12min)

| ID    | Micro-task                                                                        | Effort | Depends on |
| ----- | --------------------------------------------------------------------------------- | ------ | ---------- |
| P7.1  | Add `segment_size_stats_works_with_xchacha20_encrypted_segments` test (cfg-gated) | 12min  | —          |

---

## Execution Graph

```mermaid
graph TD
    subgraph "1% → 51% — Session cleanup + trivial wins"
        P1[P1: Archive own docs, CHANGELOG, AGENTS.md]
        P3[P3: p-{i}→prop_item, doc alias, must_use]
    end

    subgraph "4% → 64% — User-facing Display impls"
        P2[P2: Display for DurabilityPolicy, BufferStats, SegmentConfig]
    end

    subgraph "20% → 80% — Doc gaps + correctness"
        P4[P4: Examples table, Guarantees section, FEATURES inventory]
        P5[P5: compute_store_pressure + percentile property tests]
        P6[P6: seq_to_index helper extraction]
        P7[P7: XChaCha20 encrypted test]
    end

    subgraph "Verify + Commit"
        P1 --> GATE[Run full verification gate]
        P2 --> GATE
        P3 --> GATE
        P4 --> GATE
        P5 --> GATE
        P6 --> GATE
        P7 --> GATE
        GATE --> COMMIT[git commit with detailed messages]
        COMMIT --> PUSH[git push origin master]
        PUSH --> CI[Verify CI green — gh run list]
    end

    style P1 fill:#ff6b6b,color:#fff
    style P2 fill:#ffa502,color:#fff
    style GATE fill:#5352ed,color:#fff
    style CI fill:#2ed573,color:#fff
```

---

## Anti-Verschlimmbesserung Checklist

Before each code change, verify:

- [ ] Does this change published behavior? → If YES, **STOP**
- [ ] Does this touch the append/flush/read hot path? → If YES, **STOP**
- [ ] Does this change the on-disk format? → If YES, **STOP**
- [ ] Does this add a dependency? → If YES, **STOP**
- [ ] Does this change a public API signature? → If YES, **STOP** (adding trait impls is OK)
- [ ] Could this break any existing test? → If YES, **STOP**
- [ ] Is this the SIMPLEST correct solution? → If NO, **SIMPLIFY**
