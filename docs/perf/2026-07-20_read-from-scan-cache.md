# `read_from` after the v0.4.0 scan cache

**Captured:** 2026-07-20
**Method:** `cargo bench --bench bench_read_from -- --warm-up-time 1 --measurement-time 3` on the same machine as the [hot-path flamegraph](2026-07-20_hot-path-flamegraph.md). Two benchmark groups: the original `read_from` (single 10 000-item segment, varying limit) and a new `read_from_scan_cache` group that compares cold vs warm cache across 10 / 100 / 1 000 on-disk segment files.
**Caveat:** single machine, single run, criterion's standard 3 s measurement window. The 1 000-segment case showed wider confidence intervals than the others; treat those absolute numbers as indicative, not publication-grade. Re-run on your own hardware before quoting.

## TL;DR

The scan cache (added in v0.4.0, [CHANGELOG](../../CHANGELOG.md)) provides a **measurable 6–9% read-from speedup at 10 and 100 segment files**, which is the typical operating regime for the bounded-queue use case. At 1 000 segment files the cold-vs-warm delta is noisier and may even invert — the readdir cost is no longer the dominant term at that scale, so allocator and kernel-cache effects dominate. The cache is a clear win for the design intent (avoiding the double-scan on `read_from` followed by `delete_acked`); the absolute microsecond counts are not the headline.

## Existing `read_from` bench (1 segment × 10 000 items, varying limit)

| Benchmark               | Median    | Notes                                            |
| ----------------------- | --------- | ------------------------------------------------ |
| `read_from/limit_100`   | 1.4730 ms | Reads 100 of 10 000 items from one segment file. |
| `read_from/limit_1000`  | 1.3937 ms | Reads 1 000 of 10 000 items.                     |
| `read_from/limit_10000` | 1.4022 ms | Reads all 10 000 items.                          |

The three numbers are essentially identical because the dominant cost is opening + zstd-decompressing + CBOR-deserialising the **whole** segment file regardless of `limit` — `SegmentBuffer::read_segment` materialises a `Vec<T>` for the entire segment, and `read_from` then slices into it. The per-item cost after deserialisation is negligible by comparison.

**Implication for future work.** A streaming deserialiser that early-stops at `limit` would turn the ~1.4 ms above into roughly `limit / 10 000` of that (e.g. ~14 µs for `limit = 100`). That is a separate item on [TODO_LIST.md](../../TODO_LIST.md) (under "Performance") and is not addressed by the scan cache. Flagging it here because the flatness of the table above is the tell-tale signature of the all-or-nothing decode path.

## New `read_from_scan_cache` bench (N segments × 100 items, limit 100)

| Benchmark            | Median    | Cold → Warm Δ             |
| -------------------- | --------- | ------------------------- |
| `cold_10_segments`   | 33.857 µs | —                         |
| `warm_10_segments`   | 30.969 µs | **−8.5%** (cache wins)    |
| `cold_100_segments`  | 160.87 µs | —                         |
| `warm_100_segments`  | 150.96 µs | **−6.2%** (cache wins)    |
| `cold_1000_segments` | 2.2065 ms | —                         |
| `warm_1000_segments` | 2.5936 ms | +17.5% (noisy; see below) |

### Why the cache wins at 10 and 100 segments

The cold path must call `fs::read_dir` + `parse_filename` for every entry in the directory, sort the result by `start`, then clone the resulting `Vec<SegmentRange>` into the cache. The warm path skips the `read_dir` and parse; it pays only a `Vec<SegmentRange>` clone out of the cache. At 10–100 segments the `read_dir` + parse cost is a meaningful fraction of the total (the rest is opening 1 segment file, zstd-decoding it, CBOR-deserialising 100 items, and assembling the result `Vec<T>`), so removing it moves the median by 6–9%.

### Why the 1 000-segment row is noisy

At 1 000 segments the per-call cost is ~2 ms and is dominated by allocator pressure (the scan-cache clone allocates 1 000 × 16 B = 16 KB per call, which on this allocator sometimes lands in a small-object freelist and sometimes in the mmap arena) plus kernel dentry-cache effects that differ between cold (which calls `readdir` and therefore populates the dentry cache) and warm (which does not). Criterion's confidence intervals at this scale were wider than the cold-vs-warm gap, so the +17.5 % direction should be read as "no clear winner at 1 000 segments" rather than "the cache hurts".

The durable claim is the small-directory regime (10 and 100 segments), which is what `SegmentBuffer` is designed for — the bounded-queue contract means `delete_acked` is supposed to keep the directory small. A 1 000-segment directory is already signalling that the consumer is not keeping up.

### What the cache does NOT cache

The scan cache stores only the **directory listing** (`Vec<SegmentRange>`). It does **not** cache segment file contents. Every `read_from` call still pays:

- one `File::open` per segment file in the requested range,
- one zstd decompression per file,
- one CBOR deserialisation per file.

This is why the absolute microsecond counts above are not zero even on the warm path. Caching decoded segment contents is on [TODO_LIST.md](../../TODO_LIST.md) as "mtime probe for scan cache" + a follow-up design for content caching; both are deferred until a second consumer of the cache shows up.

## Comparison with the v0.1.0 baseline

The [v0.1.0-vs-v0.2.0 perf doc](2026-07-19_v0.1.0-vs-v0.2.0.md) measured `append` and `recover` but **did not** capture `read_from` numbers, so there is no direct v0.1.0-vs-HEAD comparison for reads. What we can say from the 2026-07-20 data:

- For the typical 1-segment, 10 000-item directory the v0.4.0 read path takes ~1.4 ms, dominated by whole-segment deserialisation (a known cost, not a regression).
- For the bounded-queue sweet spot (10–100 segments) the v0.4.0 scan cache trims 6–9 % off every read after the first.
- Combined with the [zstd CCtx pooling](2026-07-20_hot-path-flamegraph.md) that landed the same day, the write path is now **2× faster at small batches** while the read path holds steady with a modest cache-driven improvement.

## Reproducing

```sh
cargo bench --bench bench_read_from -- --warm-up-time 1 --measurement-time 3
```

The `read_from_scan_cache` group is part of `benches/bench_read_from.rs`; both groups run from the single `cargo bench --bench bench_read_from` invocation.
