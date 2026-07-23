# Percentile latency baseline and CI regression strategy

**Captured:** 2026-07-23
**Method:** Analysis of criterion's statistical output (`estimates.json`, `sample.json`) and the allocation-count guard (`tests/alloc_guard.rs`). No new benchmarks were run for this doc; it documents what the existing bench infrastructure already produces and how to use it for tail-latency monitoring.
**Caveat:** Criterion's percentile data is derived from 100 samples per benchmark — sufficient for detecting order-of-magnitude regressions, not for publication-grade SLA claims. Cross-machine absolute µs vary ±2–5× depending on CPU, memory, and filesystem. The allocation-count guard is the CI-stable signal; absolute µs targets are not.

## TL;DR

Criterion already records every individual sample needed to compute p99/p99.9, but does not surface them in the default terminal output (it shows mean + confidence interval). This doc shows where the data lives, how to extract it, and why the **allocation-count guard** (`tests/alloc_guard.rs`) is the machine-independent proxy that catches the most impactful tail-latency regressions in CI without hardware flakiness.

## Why percentiles, not means

DDIA's central performance lesson: in systems with concurrent background work (GC, page cache flush, dir mtime updates), the **mean is stable while the p99 can be 10–50× higher**. A `mean = 20 µs` benchmark can hide a `p99 = 800 µs` tail that causes real-world timeout cascades. The p99 is what the slowest 1 in 100 operations experience — and in a cloud-sync drain loop, that is the operation that triggers a retry, a backoff, or a stuck consumer.

## Where criterion's data lives

Each benchmark run produces four JSON files in `target/criterion/<group>/<benchmark>/new/`:

| File             | Contents                                                                                                                                                      |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `estimates.json` | Statistical estimates: `mean`, `median`, `median_abs_dev`, `std_dev`, `slope`. Confidence intervals via bootstrap.                                            |
| `sample.json`    | Raw measurement data: `times[]` (100 entries, total wall-clock per sample) and `iters[]` (iterations per sample). Per-operation time = `times[i] / iters[i]`. |
| `tukey.json`     | Tukey five-number fence for outlier detection.                                                                                                                |
| `benchmark.json` | Criterion run metadata (benchmark name, group ID, throughput unit).                                                                                           |

The percentiles are **not** in any file directly — they must be computed from `sample.json`:

```bash
# Extract p50/p90/p99/p99.9 for a benchmark (requires python3):
python3 -c "
import json, math, sys
d = json.load(open(sys.argv[1]))
per_op = sorted(t / n for t, n in zip(d['times'], d['iters']))
n = len(per_op)
for p in [50, 90, 99, 99.9]:
    idx = min(int(math.ceil(p / 100 * n)) - 1, n - 1)
    print(f'p{p}: {per_op[idx]:.0f} ns')
" target/criterion/append/batch_100/new/sample.json
```

## The CI-stable signal: allocation-count guard

Absolute µs thresholds flake on CI (hardware variance, noisy neighbors, thermal throttling). The most impactful tail-latency regressions — **extra clones, Vec growth, `format!` in hot loops** — manifest as additional heap allocations, which are hardware-independent.

The guard (`tests/alloc_guard.rs`) counts allocation events on four hot paths:

| Operation                    | Measured  | Budget | What a regression means                                           |
| ---------------------------- | --------- | ------ | ----------------------------------------------------------------- |
| Warm `append` (no flush)     | 0 allocs  | 1      | A clone or format crept into the append hot path.                 |
| `read_from(0, 50)` in-memory | 1 alloc   | 3      | The result Vec or scan path grew an extra allocation.             |
| `stats()`                    | 0 allocs  | 1      | The snapshot path is no longer allocation-free.                   |
| `append` + flush             | 27 allocs | 32     | The encode/compress/write pipeline grew a buffer or intermediate. |

**What this catches:** every regression that adds an allocation on the hot path — the class of change most likely to inflate p99. **What it does NOT catch:** algorithmic regressions that keep the allocation count flat but change cache behavior (e.g., a tighter loop with worse branch prediction). Those require the human-eyeball p99 check described below.

## Pre-release percentile check

Before tagging a release, run the benchmarks on your own machine and compare p99 to the previous release:

```bash
# 1. Run the append bench (the most latency-sensitive hot path):
cargo bench --bench bench_append

# 2. Extract percentiles:
python3 -c "
import json, math, glob
for f in sorted(glob.glob('target/criterion/append/*/new/sample.json')):
    d = json.load(open(f))
    per_op = sorted(t / n for t, n in zip(d['times'], d['iters']))
    name = f.split('/')[-3]
    for p in [50, 99]:
        idx = min(int(math.ceil(p / 100 * len(per_op))) - 1, len(per_op) - 1)
        print(f'{name} p{p}: {per_op[idx]:.0f} ns')
"

# 3. If p99 regressed > 2× from the previous release, investigate before tagging.
```

## Relationship to other perf docs

- [Hot-path flamegraph](2026-07-20_hot-path-flamegraph.md) — identifies where CPU time goes (zstd + CBOR dominate; allocation overhead is visible but secondary).
- [`read_from` scan cache](2026-07-20_read-from-scan-cache.md) — shows the scan cache's median improvement (6–9%); the p99 improvement was within the noise.
- [Scaling and payload entropy sweep](2026-07-21_scaling-and-payload-entropy-sweep.md) — end-to-end throughput at 1M–100M scale; throughput is throughput, not latency, but the allocation-count trends correlate.
