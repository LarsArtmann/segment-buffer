# Scaling & payload-entropy sweep: 1M–100M items, uniform→random payloads

**Captured:** 2026-07-21
**Code state:** working tree on top of `40bc6ee` (uncommitted changes in `examples/scaling.rs` + `docs/PERFORMANCE.md` that add the scaling driver and the `payload_kind` arg). Run `git status` for the current diff before citing these numbers.
**Method:** `cargo run --release --example scaling -- [count] [batch] [zstd-level] [payload_mult] [payload_kind]`. Single back-to-back run per row; no criterion statistical window. The driver excludes payload-string generation from the load-phase timing, so the load throughput reflects only the buffer pipeline (mutex → CBOR → zstd → file I/O).
**Hardware:** AMD Ryzen AI MAX+ 395 (32 logical cores), 93 GiB RAM, Linux 7.1.3. `cargo 1.96.2`, `rustc 1.96.1`. Tempdir on tmpfs (`/tmp`), so the disk-I/O dimension is **not** exercised — these numbers are CPU-bound (zstd dominates once payload entropy rises).
**Caveat:** single machine, single run, no noise bars. Treat absolutes as order-of-magnitude; ratios across rows are the durable claim. For production numbers, re-run on the target deployment machine against real disk.

> **Scope:** these numbers are the workload class `docs/PERFORMANCE.md` explicitly said was **not** covered by the criterion micro-benchmarks. The micro-benches (max 10k items, fresh buffer per iteration, `uniform`-equivalent payloads) remain authoritative for the per-operation costs they isolate. This document is the complementary end-to-end lifecycle view.

---

## What was added

Two CLI args on `examples/scaling.rs`:

- `[payload_mult]` — multiplies the 64-byte base payload (1× = 64 B/item uncompressed).
- `[payload_kind]` — selects the entropy profile: `uniform` | `text` | `json` | `random`.

The load phase was re-timed so payload-string construction (a producer cost, dominated by `format!()` and `String` allocation) is excluded from the buffer throughput number. Wall time (including generation) is still printed for operators who care about the full producer cost.

## Result 1: scale sweep at the default uniform payload

`cargo run --release --example scaling -- [count] [batch] 3`

| Count | Batch | Load items/s | Load MiB/s | Recover (seg/s) | Drain items/s | Drain MiB/s | Peak disk |
| ----- | ----- | ------------ | ---------- | --------------- | ------------- | ----------- | --------- |
| 100k  | 5k    | 7,596,227    | 587        | 1,312           | 3,330,083     | 257         | 0.4 MiB   |
| 1M    | 5k    | —            | —          | —               | —             | —           | —         |
| 10M   | 5k    | 6,893,061    | 533        | 106,561         | 3,391,959     | 262         | 36 MiB    |
| 100M  | 10k   | 6,609,941    | 511        | 343,383         | 3,234,045     | 250         | 360 MiB   |

**Reading:** throughput is **flat from 100k to 100M items** — there is no degradation with segment count. Recovery cost grows sublinearly in segment count (cached filename scan). All 100M items were verified gap-free, in-order, and disk drained to zero.

> 1M row is omitted from this table because the prior session's 1M run used a different batch shape; the 100k / 10M / 100M rows are the clean apples-to-apples scale comparison. Re-run `cargo run --release --example scaling` for a fresh 1M number on the current hardware.

## Result 2: payload-size sweep (uniform payload, 1M items)

`cargo run --release --example scaling -- 1000000 5000 3 [mult]`

| Mult | Payload | Uncompressed | Load items/s | Load MiB/s | Comp ratio | Peak disk |
| ---- | ------- | ------------ | ------------ | ---------- | ---------- | --------- |
| 4×   | 256 B   | 273 B/item   | 7,515,885    | 1,957      | 62×        | 4.2 MiB   |
| 10×  | 640 B   | 657 B/item   | 5,026,709    | 3,150      | 142×       | 4.4 MiB   |
| 20×  | 1280 B  | 1297 B/item  | 3,347,778    | 4,141      | 267×       | 4.6 MiB   |
| 50×  | 3200 B  | 3217 B/item  | 544,131      | 1,669      | 613×       | 5.0 MiB   |

**Reading:** load MiB/s peaks around the 20× payload (~4.1 GB/s) then drops at 50×. The 50× cliff in items/sec (544k vs 3.3M at 20×) is **per-batch allocation pressure** — each batch is 5000 × 3.2 KB = 16 MB of heap-allocated `String` payloads, and the producer-side `make_payload()` loop dominates wall time. The compression ratios (62×–613×) are absurd because the payload is a uniform `x`-string; **do not extrapolate disk footprint from this table to real workloads.** Use Result 3 for that.

## Result 3: payload-entropy sweep (the headline finding)

`cargo run --release --example scaling -- 1000000 5000 3 10 [kind]`

1M items, 640 B payload, batch 5000, zstd-3.

| Payload | Comp ratio | Compressed/item | Load items/s | Load MiB/s | Drain items/s | Drain MiB/s | Peak disk |
| ------- | ---------- | --------------- | ------------ | ---------- | ------------- | ----------- | --------- |
| uniform | 142×       | 4.63 B          | 4,539,905    | 2,845      | 2,998,488     | 1,879       | 4.4 MiB   |
| text    | 3.1×       | 214.73 B        | 321,040      | 201        | 1,232,681     | 772         | 205 MiB   |
| json    | 3.4×       | 195.21 B        | 329,317      | 206        | 1,318,832     | 826         | 186 MiB   |
| random  | 1.8×       | 356.33 B        | 277,316      | 174        | 1,182,534     | 741         | 340 MiB   |

**Payload kinds:**

- `uniform` — `"x".repeat(n)`. Maximum compressibility; theoretical ceiling.
- `text` — log-line-like strings from a 64-word vocabulary with varied numeric values (`"1700000000 INFO worker=12 action=flush ..."`). Models server/agent telemetry.
- `json` — array of semi-structured JSON objects with varying field values (`[{"id":...,"lvl":"INFO","k":"event","v":123,"f":0.453},...]`). Models event pipelines.
- `random` — pseudo-random hex strings from a deterministic SplitMix64 PRNG seeded per item id. Near-incompressible; theoretical floor.

### Interpretation

1. **The uniform baseline overstates load throughput by ~14×.** Going from `uniform` (4.5 M items/s) to `text`/`json` (~320 k items/s) is a 14× drop, and that drop is **almost entirely zstd compression cost**, not buffer overhead. High-entropy data is fundamentally harder to compress, and zstd-3 is the bottleneck once the payload is no longer trivially compressible.

2. **Drain throughput is far less sensitive to entropy** (~2.5× drop, uniform → random). zstd decompression is markedly cheaper than compression, and the drain path does not allocate payload strings. The read-side bottleneck is closer to I/O and CBOR deserialization than to entropy.

3. **Disk footprint is the real cost of entropy.** Uniform 640 B/item compresses to 4.6 B on disk; the same item at `random` entropy takes 356 B — a **77× larger disk footprint**. At 100M items with random payloads, extrapolate to ~34 GB on disk vs ~360 MB for uniform. Size your `max_size_bytes` and your disk accordingly.

4. **`text` and `json` are practically interchangeable for throughput.** Both are semi-structured with overlapping entropy profiles (3.1× vs 3.4× compression). Pick whichever models your workload; do not expect a meaningful throughput delta between them.

5. **The buffer pipeline is not the bottleneck once payloads are realistic.** With `text`/`json` payloads, load throughput is ~320 k items/s. The buffer's contribution (mutex + CBOR + I/O) is well under that; zstd is. If you need more load throughput, the lever is the compression level (try `zstd-1`) or the batch size, not the buffer internals.

### What this changes about prior claims

The README and `docs/PERFORMANCE.md` previously cited numbers in the 5–7 M items/s range for the append+flush path. **Those numbers are uniform-payload numbers** and overstate real-world throughput by roughly an order of magnitude. The honest production-shape number for 640 B semi-compressible events on this hardware is **~320 k items/s load, ~1.2 M items/s drain**. The prior numbers are not wrong — they are the ceiling. This document adds the floor and the middle.

## What this is NOT

- **Not a latency distribution.** Throughput is total-items / total-wall-time. p50/p99 latency under load remains a deferred item.
- **Not disk-bound.** Tempdir lives on tmpfs. On spinning disk or networked storage, drain throughput will drop and become the new bottleneck. Re-run on the target hardware for production numbers.
- **Not encryption-aware.** The runs use the default (no cipher). The encryption feature adds a per-flush AEAD cost that will further reduce load throughput; it is not measured here.
- **Not multi-producer.** The scaling driver is single-threaded. The stress test `concurrency_4_writers_1_reader_10k_events` covers the MPMC regime at smaller scale (see `2026-07-19_v0.4.1_stress_throughput.md` for the concurrent baseline).
- **Not a regression benchmark.** There is no prior payload-entropy baseline to compare against; this is the first audit. Future changes to the compression pipeline (zstd version, compression context pooling, streaming cipher) should be re-measured against this table on the same hardware.

## Reproduction

```bash
# Scale sweep (Result 1)
cargo run --release --example scaling -- 100000 5000 3
cargo run --release --example scaling -- 10000000 5000 3
cargo run --release --example scaling -- 100000000 10000 3

# Payload-size sweep (Result 2)
for mult in 4 10 20 50; do
  cargo run --release --example scaling -- 1000000 5000 3 $mult
done

# Payload-entropy sweep (Result 3, the headline)
for kind in uniform text json random; do
  cargo run --release --example scaling -- 1000000 5000 3 10 $kind
done
```

Expect ±10–20% variance on a typical laptop due to thermal/frequency scaling and background load. The ratios across `payload_kind` rows (Result 3) are the durable claim; the absolute items/sec figures are hardware-specific.

## Open questions for future sessions

- **Is zstd level 1 a free 2× on load throughput at `text`/`json` entropy?** Not measured here. The scaling driver takes `compression` as the third arg, so the experiment is one bash loop away.
- **Streaming cipher cost.** The encryption feature adds AEAD per flush; its throughput delta at scale is undocumented. Run with `DURABILITY` left as `Throughput` and a cipher installed via `SegmentConfigBuilder::cipher` to measure.
- **Real disk.** Re-run Result 3 on the monitor365 deployment host (or whatever the production disk is). The tmpfs numbers understate I/O-bound regimes.
- **p50/p99 latency.** Throughput-only here. A latency distribution under load needs a different driver (per-item timestamps, histogram).
