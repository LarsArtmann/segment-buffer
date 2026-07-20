# Performance methodology

How segment-buffer measures performance, how to reproduce the numbers, and how
to interpret the noise.

## Controlled baselines

The headline comparisons (e.g. "append 30–65% slower vs v0.1.0") come from a
controlled `git worktree` baseline:

1. Check out the reference tag in a separate worktree: `git worktree add ../sb-baseline v0.1.0`.
2. Build the same criterion bench in both worktrees.
3. Run each bench with the same sample size on the same machine, back-to-back.
4. Capture the median (criterion's point estimate).

The raw results live in [`perf/`](./perf/) with the date and the versions
compared. Each file is a point-in-time snapshot — it is not auto-refreshed when
new code lands.

## Reproducing

```bash
# Build and run a specific bench
cargo bench --bench bench_append --features encryption

# Compare two versions
git worktree add ../sb-baseline v0.1.0
(cd ../sb-baseline && cargo bench --bench bench_append --features encryption)
cargo bench --bench bench_append --features encryption
# Compare the two criterion HTML reports under target/criterion/<bench>/new/
```

The benches live in [`benches/`](../benches/) and use `criterion` with
`iter_with_setup` so the per-iteration cost reflects only the operation under
test, not the buffer construction.

## Available benchmarks

| Bench                     | What it measures                                                                                       |
| ------------------------- | ------------------------------------------------------------------------------------------------------ |
| `bench_append`            | Append throughput at batch sizes 1, 100, 1k, 10k                                                       |
| `bench_read_from`         | `read_from` across flushed + in-memory items (incl. cold-vs-warm `read_from_scan_cache` group, v0.4.0) |
| `bench_read_vs_for_each`  | `read_from` vs `for_each_from` (lending iterator) on 1k items                                          |
| `bench_delete_acked`      | `delete_acked` at 100 and 10k segments                                                                 |
| `bench_recover`           | Cold-start recovery over a populated directory                                                         |
| `bench_stats`             | `stats()` snapshot vs 3 individual accessors                                                           |
| `bench_append_all`        | `append_all` batch primitive vs loop of `append`                                                       |
| `bench_durability_policy` | _(v0.5.0)_ A/B/C `Maximal` vs `Segment` vs `Throughput` on a 1000-event flush                          |

## Interpreting the numbers

### Single-run, single-machine

Unless explicitly stated otherwise, every number in this repo is a single-run
median from one developer machine. There are no statistical noise bars, no
multi-machine matrix, no p99 confidence intervals. The numbers are
**indicative of direction, not publication-grade**. A 30% delta is real; a 3%
delta is noise.

### Relative ratios are the durable claim

Absolute nanosecond counts are hardware-dependent and rot the moment the bench
moves to a different CPU. The durable claims are **ratios**: "`stats()` is
~2.5× cheaper than 3 individual accessors", "`for_each_from` is ~21× faster
than `read_from` on in-memory items". Ratios hold across hardware in
proportion; absolutes do not.

### What the envelope costs

Every segment write prepends an 8-byte `SBF1` envelope. On large batches this
is amortized to nothing; on single-item appends it is a measurable fraction of
the per-write cost. The "append 30–65% slower vs v0.1.0" headline is dominated
by this effect plus the stats bookkeeping that v0.2.0 introduced. The
`FlushPolicy::Manual` + `append_all` path (v0.4.1) recovers most of this for
bulk-load workloads by amortizing the lock + bookkeeping across the whole
batch.

## When to re-bench

- After any change to the hot path (`append`, `flush`, `read_from`).
- After a dependency bump (`zstd`, `ciborium`, `parking_lot`).
- Before cutting a release that cites a perf number in the CHANGELOG.
- When a claim in this repo says "~Nx faster" and you suspect it has drifted.

## What is NOT measured here

- **Real-world throughput** under a specific workload. The benches are
  micro-benchmarks, not end-to-end pipeline tests.
- **Memory allocation patterns.** Use `cargo flamegraph` or `dhat` for that.
- **Disk I/O variance.** The benches use `tempfile` (typically tmpfs on CI),
  which hides real disk latency. Production numbers will differ.
