# Compression Level Sweep Analysis

**Date:** 2026-08-10
**Updated with analysis:** 2026-08-11
**Data:** [`2026-08-10_compression-level-sweep.tsv`](./2026-08-10_compression-level-sweep.tsv)
**Crate version at data collection:** v0.5.7 (compression default already changed to level 1)
**Machine:** single-run, tmpfs-backed, release build

---

## Methodology

The `scaling` example was run at each zstd compression level (1–22) with three
payload kinds: **uniform** (highly compressible synthetic data), **text**
(realistic English prose), and **json** (structured JSON objects). Load =
`append_all` + `flush` throughput. Drain = `read_from` throughput. 10 000 items
per batch, `DurabilityPolicy::Throughput`.

Full methodology and raw TSV: see the scaling example source
(`examples/scaling.rs`).

---

## Key findings

### 1. The level-1 default (shipped v0.5.7) is the right choice for cloud-sync workloads

For uniform payloads, level 1 achieves **5.9M items/s** load throughput with a
**142x** compression ratio. Level 3 (the old default) achieves only **3.4M
items/s** — a **42% throughput penalty** — for the same compression ratio (also
142x). Higher levels eventually reach 183x, but at a 5–10x throughput cost.

For text and JSON payloads (more representative of real cloud-sync data), the
tradeoff is even clearer:

| Payload | Level | Load ips | Load MiB/s | Ratio | vs level 1 throughput |
| ------- | ----- | -------- | ---------- | ----- | --------------------- |
| text    | 1     | 800 657  | 501.7      | 3.2x  | baseline              |
| text    | 3     | 401 710  | 251.7      | 3.1x  | 2.0x slower           |
| text    | 5     | 169 255  | 106.0      | 3.2x  | 4.7x slower           |
| json    | 1     | 701 053  | 439.3      | 3.6x  | baseline              |
| json    | 3     | 376 855  | 236.1      | 3.4x  | 1.9x slower           |
| json    | 5     | 131 685  | 82.5       | 3.5x  | 5.3x slower           |

The compression ratio barely changes between levels 1 and 5 for real-world
payloads (3.1x–3.5x), while throughput drops 5x. The extra CPU spent on higher
compression is wasted — the cloud endpoint is the durable copy.

### 2. Drain (read) throughput is compression-level-independent

Drain throughput stays remarkably flat across all levels:

| Payload | Level 1 drain | Level 10 drain | Level 22 drain |
| ------- | ------------- | -------------- | -------------- |
| uniform | 2.98M ips     | 3.10M ips      | 3.19M ips      |
| text    | 1.61M ips     | 1.51M ips      | 1.61M ips      |
| json    | 1.51M ips     | 1.20M ips      | 1.73M ips      |

zstd decompression is O(compressed_size), not O(level). The consumer's read
path is unaffected by the producer's compression-level choice.

### 3. The "knee" is at level 5–6 for uniform payloads

Uniform data compresses so well that zstd finds the patterns almost
immediately. Levels 1–5 all achieve ~142x at 3.5–5.9M ips. At level 6, the
ratio jumps to 183x but throughput drops to ~1M ips. For text/json there is no
knee — throughput degrades smoothly.

### 4. Levels above 15 are impractical

At level 16+, uniform-payload load throughput drops below 350K ips with no
additional compression benefit over level 15. Text/json are even worse
(<4K ips). These levels exist for archival use cases, not real-time buffering.

### 5. Peak disk usage varies with compression ratio

Higher compression means smaller segment files:

| Payload | Level 1 peak disk | Level 10 peak disk | Level 22 peak disk |
| ------- | ----------------- | ------------------ | ------------------ |
| uniform | 4.4 MiB           | 1.7 MiB            | 2.0 MiB            |
| text    | 194.1 MiB         | 186.3 MiB          | 158.8 MiB          |
| json    | 173.3 MiB         | 171.4 MiB          | 142.7 MiB          |

For uniform data the disk savings are dramatic (4.4 → 1.7 MiB). For realistic
payloads the savings are modest (~10%) and not worth the throughput cost.

---

## Recommendation

**Use level 1 (the default).** The compression ratio for real-world payloads
differs by <10% between levels 1 and 5, while throughput differs by 5x. The
cloud endpoint holds the durable copy — the local buffer should prioritise
throughput over compression savings.

**Override to level 3–5** if disk space is constrained and the workload is
uniform/compressible. The throughput penalty is ~2x but disk savings can be
significant for highly compressible data.

**Never use levels above 10** for real-time buffering. The throughput collapse
(>100x slower) makes them unsuitable for any producer-side buffer.
