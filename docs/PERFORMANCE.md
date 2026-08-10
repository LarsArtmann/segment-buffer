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
new code lands. The most recent end-to-end scaling + payload-entropy snapshot
is [`2026-07-21_scaling-and-payload-entropy-sweep.md`](./perf/2026-07-21_scaling-and-payload-entropy-sweep.md)
— read it before quoting any items/sec headline, because the uniform-payload
baselines overstate real-world throughput by roughly an order of magnitude.

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
| `bench_read_vs_for_each`  | `read_from` vs `for_each_from` (callback iterator) on 1k and 10k items                                 |
| `bench_delete_acked`      | `delete_acked` at 100 and 10k segments                                                                 |
| `bench_recover`           | Cold-start recovery over a populated directory                                                         |
| `bench_stats`             | `stats()` snapshot vs 3 individual accessors                                                           |
| `bench_append_all`        | `append_all` batch primitive vs loop of `append`                                                       |
| `bench_durability_policy` | _(v0.5.0)_ A/B/C `Maximal` vs `Segment` vs `Throughput` on a 1000-event flush                          |
| `bench_segment_size_stats`| _(v0.5.5)_ `segment_size_stats()` scan cost at 100, 1k, 10k segments                                   |
| `bench_cipher`            | _(v0.5.0)_ Flush encode pipeline: no cipher vs AES-256-GCM vs XChaCha20-Poly1305 (requires `--features encryption`) |

## Scaling test (end-to-end, 1M–100M scale)

The criterion benches above are micro-benchmarks (max 10k items, fresh buffer
per iteration). For real-world scaling — the full cloud-sync lifecycle at
millions of items — run the standalone scaling driver:

```bash
cargo run --release --example scaling                                     # 1M, batch 5000, zstd-3, 64B, uniform
cargo run --release --example scaling -- 10000000                         # 10M
cargo run --release --example scaling -- 100000000 10000 1                # 100M, batch 10k, zstd-1
cargo run --release --example scaling -- 1000000 5000 3 10 text           # 1M, 10x payload, semi-compressible text
cargo run --release --example scaling -- 1000000 5000 3 10 random         # 1M, 10x payload, pseudo-random hex
```

Args: `[count] [batch_size] [compression] [payload_mult] [payload_kind]`.

It runs three timed phases — **load** (`append_all` + `flush`, payload
generation excluded from timing), **recover** (drop + reopen), **drain**
(`read_from` + `delete_acked`) — and verifies sequence integrity (gap-free,
in-order, exactly `count` items, disk drained to zero) at the end. Throughput
is reported as items/sec and uncompressed MiB/sec per phase, plus segment
count, compression ratio, and recovery cost.

### Payload kinds and why they matter

The `payload_kind` arg selects the entropy of the payload, which dominates
both the compression ratio and the CPU cost of zstd:

| kind      | typical zstd ratio | models                               | load throughput         |
| --------- | ------------------ | ------------------------------------ | ----------------------- |
| `uniform` | 50-600x            | uniform fill — best-case ceiling     | highest (unrealistic)   |
| `text`    | 3-6x               | log-line-like telemetry              | ~14x lower than uniform |
| `json`    | 3-5x               | semi-structured event pipeline       | ~14x lower than uniform |
| `random`  | ~1.1x              | pseudo-random hex — worst-case floor | ~16x lower than uniform |

**The uniform baseline overstates throughput by ~14×.** zstd compression of
high-entropy data is the dominant cost, not the buffer pipeline. Always
benchmark with `text` or `json` (whichever models your workload) for a
production-representative number.

This is **not** part of the verification gate (it takes 15–45s at 100M scale
and needs real disk). Run it on the target deployment machine for numbers that
reflect production. The `Throughput` durability policy is used by default
(cloud-sync deployment); edit the `DURABILITY` constant in
`examples/scaling.rs` to measure the fsync-bound `Maximal`/`Segment` regime.

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
~2.5× cheaper than 3 individual accessors". Ratios hold across hardware in
proportion; absolutes do not.

### What the envelope costs

Every segment write prepends an 8-byte `SBF1` envelope. On large batches this
is amortized to nothing; on single-item appends it is a measurable fraction of
the per-write cost. The v0.1.0→v0.2.0 "30–65% slower" headline was real at the
time, but the 2026-07-20 PGO session (see
[`perf/2026-07-20_hot-path-flamegraph.md`](./perf/2026-07-20_hot-path-flamegraph.md))
pooled the zstd `CCtx` and made the crate **~2.3× faster than v0.1.0** on
small batches — the old regression is more than reversed. The
`FlushPolicy::Manual` + `append_all` path (v0.4.1) recovers further for
bulk-load workloads by amortizing the lock + bookkeeping across the whole
batch.

## Tuning for your workload

The crate's target use case is the local throughput buffer in front of cloud
sync. The cloud endpoint is normally the bottleneck; the levers below are for
producers whose append or drain rate is gated by this buffer locally. They are
all config-only — no code change, no format change, no new dependency.

### 1. `DurabilityPolicy::Throughput` (biggest single win)

The default `DurabilityPolicy::Segment` fsyncs the segment file's data on every
flush. `Throughput` removes the fsync entirely. For cloud-sync deployments
where the cloud endpoint holds the durable copy, this is the correct default —
the local disk is a throughput buffer, not the system of record.

```rust
use segment_buffer::{DurabilityPolicy, SegmentConfig};

let config = SegmentConfig::builder()
    .durability(DurabilityPolicy::Throughput)
    .build();
```

**When NOT to use `Throughput`:** when this buffer IS the last copy of the data
(standalone queue deployments). Use `Maximal` instead — it fsyncs both the file
and the directory inode after rename, closing the ~5–30s rename-window gap that
`Segment` leaves open. See the README "Crash behavior" table for the full
policy matrix.

### 2. `FlushPolicy::Manual` + `append_all` (amortize the flush path)

The default `FlushPolicy::Batch(1000)` auto-flushes when the in-memory batch
crosses the threshold. The threshold-crossing append pays the full
CBOR → zstd → cipher → `write_atomic` cost inline.

For bulk-load workloads (a producer that appends in bursts), `Manual` flush
policy + `append_all` amortizes the lock acquisition, encode, and file creation
across the whole batch:

```rust
use segment_buffer::{FlushPolicy, SegmentConfig};

let config = SegmentConfig::builder()
    .flush_policy(FlushPolicy::Manual)
    .build();

// Append a full batch under one lock acquisition, then flush once.
buffer.append_all(items)?;
buffer.flush()?;
```

`append_all` assigns contiguous sequence numbers under a single mutex
acquisition; `flush()` then writes one segment file. This beats N individual
`append()` calls (N lock acquisitions) when the producer can batch.

> **Low-throughput producers:** if you can't call `append_all` (items arrive
> one at a time from a slow source), `FlushPolicy::BatchOrIntervalMin` is the
> write-amplification alternative. It gates interval-triggered flushes on a
> `min_batch` threshold, so a trickle of events groups into fewer, larger
> segments instead of producing a tiny 1-event segment every `interval`.
> The `max_interval` safety valve bounds crash-recovery latency:
>
> ```rust
> use segment_buffer::{FlushPolicy, SegmentConfig};
> use std::time::Duration;
>
> let config = SegmentConfig::builder()
>     .flush_at_batch_or_interval_min(256, 10, Duration::from_secs(5), Duration::from_secs(60))
>     .build();
> // Flush at 256 items; every 5s only if 10+ pending; every 60s regardless.
> ```

### 3. `compression_level` tuning

The default zstd level is **1** (fastest encode). The compression-level sweep
(see `docs/perf/2026-08-10_compression-level-sweep.tsv`) showed level 1 is
**2x faster** than level 3 on realistic payloads with negligible ratio loss
(3.2x vs 3.1x for text). For a throughput buffer where segments are short-lived,
encode speed matters more than ratio.

```rust
// Default is already 1 — override only if you need higher ratio:
let config = SegmentConfig::builder()
    .compression_level(3)
    .build();
```

Range is 1-22; higher levels trade encode speed for ratio. Levels above 10
collapse load throughput to single-digit K items/s for negligible ratio gain
(level 22 on text: 3.9x ratio but 200x slower than level 1).

### 4. `for_each_from` vs `read_from` (callback vs owned `Vec<T>`)

`read_from(start, limit)` returns an owned `Vec<T>`. `for_each_from(start,
limit, callback)` invokes your callback once per item and avoids returning an
owned `Vec<T>`. Since the panic-free re-entrancy fix, both paths clone the
in-memory tail once — `for_each_from` snapshots the window under the lock and
releases it before the callback. The two are now roughly equal on in-memory
items (~23 us at 1k); `for_each_from` stays marginally cheaper because it
avoids the returned `Vec<T>` allocation and drop. Re-entrant calls from inside
the callback (`append`, `stats`, `delete_acked`) are safe — no panic, no
deadlock.

```rust,ignore
buffer.for_each_from(0, 1000, |seq, item| {
    // Your drain logic — no returned Vec<T> to allocate or drop.
    // Re-entry into the buffer here is safe (panic-free).
})?;
```

Use `read_from` when you need to own the items (e.g. sending across a thread
boundary); use `for_each_from` for callback-style in-place processing (e.g.
serializing to a cloud request body).

### Ordering of impact

For the cloud-sync deployment target, the levers rank roughly:

1. `Throughput` — removes fsync from every flush (the single biggest constant).
2. `Manual` + `append_all` — amortizes the flush path for bulk producers.
3. `for_each_from` — removes allocation from the drain path.
4. `compression_level(1)` — shaves encode time; marginal vs the above.

If the cloud endpoint is the bottleneck (the common case), even lever 1 alone
is enough — the buffer is no longer on the critical path.

## Baseline snapshot (2026-08-10, v0.5.6, `--features encryption`)

Single-run, single-machine, tmpfs-backed. Absolute numbers are indicative;
ratios are durable. Run `cargo bench --features encryption` to reproduce.

### Append throughput

| Benchmark              | Median     | Throughput   |
| ---------------------- | ---------- | ------------ |
| `append/batch_1`       | 27.0 µs    | 37 Kelem/s   |
| `append/batch_100`     | 48.4 µs    | 2.07 Melem/s |
| `append/batch_1000`    | 157.4 µs   | 6.35 Melem/s |
| `append/batch_10000`   | 1.19 ms    | 8.40 Melem/s |
| `append_all/100`       | 45.0 µs    | 2.22 Melem/s |
| `append_all/1000`      | 133.9 µs   | 7.47 Melem/s |
| `append_all/10000`     | 1.19 ms    | 8.39 Melem/s |

### Read path

| Benchmark                          | Median     | Throughput   |
| ---------------------------------- | ---------- | ------------ |
| `read_from/limit_100`              | 1.51 ms    | 66 Kelem/s   |
| `read_from/limit_1000`             | 1.45 ms    | 691 Kelem/s  |
| `read_from/limit_10000`            | 1.50 ms    | 6.68 Melem/s |
| `read_from_scan_cache/cold_10`     | 68.8 µs    | 1.45 Melem/s |
| `read_from_scan_cache/warm_10`     | 51.4 µs    | 1.94 Melem/s |
| `read_from_scan_cache/cold_100`   | 220.5 µs   | 453 Kelem/s  |
| `read_from_scan_cache/warm_100`    | 230.4 µs   | 434 Kelem/s  |
| `read_from_scan_cache/cold_1000`  | 2.74 ms    | 36.5 Kelem/s |
| `read_from_scan_cache/warm_1000`  | 2.32 ms    | 43.1 Kelem/s |

### `read_from` vs `for_each_from`

| Benchmark                          | Median     |
| ---------------------------------- | ---------- |
| `read_vs_for_each/read_from/1000`     | 18.0 µs  |
| `read_vs_for_each/for_each_from/1000` | 17.6 µs  |
| `read_vs_for_each/read_from/10000`    | 183.9 µs |
| `read_vs_for_each/for_each_from/10000` | 196.4 µs |

### Delete, recovery, stats

| Benchmark                    | Median     | Throughput   |
| ---------------------------- | ---------- | ------------ |
| `delete_acked/100_segments`  | 252.4 µs   | 396 Kelem/s  |
| `delete_acked/10k_segments`  | 36.77 ms   | 272 Kelem/s  |
| `recover/10_segments`         | 15.15 ms   | 660 elem/s   |
| `recover/100_segments`        | 15.57 ms   | 6.42 Kelem/s |
| `recover/1000_segments`       | 18.72 ms   | 53.4 Kelem/s |
| `stats/stats_snapshot`       | 12.95 ns   | —            |
| `stats/individual_accessors`  | 20.55 ns   | —            |

### Durability policy (1000-event flush)

| Policy        | Median     | Throughput   |
| ------------- | ---------- | ------------ |
| `Maximal`     | 198.5 µs   | 5.04 Melem/s |
| `Segment`     | 174.4 µs   | 5.74 Melem/s |
| `Throughput`  | 193.6 µs   | 5.17 Melem/s |

### Cipher overhead (flush encode pipeline)

| Cipher               | Median     |
| -------------------- | ---------- |
| no cipher            | 51.2 µs    |
| AES-256-GCM          | 54.8 µs    |
| XChaCha20-Poly1305   | 61.7 µs    |

### Segment size stats scan cost

| Segments | Median     |
| -------- | ---------- |
| 100      | 47.1 µs    |
| 1000     | 464.4 µs   |
| 10000    | 6.18 ms    |

## When to re-bench

- After any change to the hot path (`append`, `flush`, `read_from`).
- After a dependency bump (`zstd`, `ciborium`, `parking_lot`).
- Before cutting a release that cites a perf number in the CHANGELOG.
- When a claim in this repo says "~Nx faster" and you suspect it has drifted.

## What is NOT measured here

- **Statistical rigor.** Both the benches and the scaling test are single-run,
  single-machine numbers. There are no noise bars, no multi-machine matrix, no
  p99 confidence intervals. Ratios are durable; absolutes are indicative.
- **Memory allocation patterns.** Use `cargo flamegraph` or `dhat` for that.
- **Disk I/O variance on real hardware.** `cargo test` and the default bench
  setup use `tempfile` (often tmpfs), which hides real disk latency. The
  scaling test (`cargo run --release --example scaling`) closes this gap for
  end-to-end lifecycle throughput, but micro-bench numbers still reflect tmpfs.
  Production numbers on spinning disk or networked storage will differ.
