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
| `bench_read_vs_for_each`  | `read_from` vs `for_each_from` (lending iterator) on 1k items                                          |
| `bench_delete_acked`      | `delete_acked` at 100 and 10k segments                                                                 |
| `bench_recover`           | Cold-start recovery over a populated directory                                                         |
| `bench_stats`             | `stats()` snapshot vs 3 individual accessors                                                           |
| `bench_append_all`        | `append_all` batch primitive vs loop of `append`                                                       |
| `bench_durability_policy` | _(v0.5.0)_ A/B/C `Maximal` vs `Segment` vs `Throughput` on a 1000-event flush                          |

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
~2.5× cheaper than 3 individual accessors", "`for_each_from` is ~21× faster
than `read_from` on in-memory items". Ratios hold across hardware in
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

### 3. `compression_level(1)` (faster encode, marginal ratio loss)

The default zstd level is **3** (fast with a good ratio). Level **1** is
roughly 2–3× faster to encode with a marginal compression-ratio loss. For a
local throughput buffer where ratio is secondary — segments are short-lived,
drained to the cloud and deleted within hours — the encode speed often matters
more:

```rust
let config = SegmentConfig::builder()
    .compression_level(1)
    .build();
```

Range is 1–22; higher levels trade encode speed for ratio. Level 1 is the
practical floor for fastest encode; levels above ~10 are rarely worth the
encode cost for a spooling buffer.

### 4. `for_each_from` over `read_from` (drain-side hot path)

`read_from(start, limit)` returns a `Vec<T>` — it allocates and clones every
item. `for_each_from(start, limit, callback)` is a lending iterator that hands
each `&(seq, T)` to your callback with **zero allocation** on the in-memory
path. On 1k in-memory items, `for_each_from` is roughly **21× faster** than
`read_from` (see [FEATURES.md](../FEATURES.md)).

```rust,ignore
buffer.for_each_from(0, 1000, |(seq, item)| {
    // Your drain logic — no Vec allocation, no clone.
    // Re-entry into the buffer here panics (re-entrancy guard).
})?;
```

Use `read_from` when you need to own the items (e.g. sending across a thread
boundary); use `for_each_from` when you process them in place (e.g. serializing
to a cloud request body).

### Ordering of impact

For the cloud-sync deployment target, the levers rank roughly:

1. `Throughput` — removes fsync from every flush (the single biggest constant).
2. `Manual` + `append_all` — amortizes the flush path for bulk producers.
3. `for_each_from` — removes allocation from the drain path.
4. `compression_level(1)` — shaves encode time; marginal vs the above.

If the cloud endpoint is the bottleneck (the common case), even lever 1 alone
is enough — the buffer is no longer on the critical path.

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
