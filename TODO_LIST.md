# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
This file tracks only work that is **not** blocked on a format change or a
missing concrete consumer. Long-term vision and raw ideas (async I/O,
envelope v2, second `SegmentStore` impl, streaming cipher, parallel flush
workers) live in [ROADMAP.md](ROADMAP.md); shipped work lives in
[CHANGELOG.md](CHANGELOG.md).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (recent entries
stay until the next CHANGELOG cut, then move out).

---

## Performance

The crate's target use case is the local throughput buffer in front of cloud
sync (see [AGENTS.md](AGENTS.md) "Product positioning"). The cloud endpoint is
usually the bottleneck, but for append-heavy producers the local flush path
can become the limiter. These items decouple writers from that path **without
changing the on-disk format** — none are blocked on envelope v2.

- [ ] **Background flush worker** — move the CBOR → zstd → cipher →
      `write_atomic` pipeline off the append thread onto a dedicated worker
      fed by a bounded channel. Today `append()` runs `flush()` inline when
      the `FlushPolicy` threshold is crossed, so the threshold-crossing
      writer pays the full encode + I/O cost. A background worker lets that
      writer hand off and return, bounded only by the channel depth.
      Confirmed greenfield: `rg 'spawn|channel|mpsc' src/` returns only test
      code today. Hard parts: backpressure when append rate exceeds flush
      rate (the crate ships no admission policy — see AGENTS.md "No
      backpressure policy"); drain-on-drop ordering (a dropped buffer must
      finish in-flight flushes before releasing the `flock`).
- [ ] **`unflushed` Vec recycling** — `flush()` calls
      `std::mem::take(&mut inner.unflushed)`, leaving `unflushed` as a
      zero-capacity `Vec`. The subsequent `append()` calls realloc as the
      Vec grows back. Return the taken Vec to a pool (or `reserve` the
      replacement) to remove the per-flush realloc on high-frequency flush
      workloads. Small, bounded, no public API change.
- [ ] **Performance tuning guide** — `docs/PERFORMANCE.md` covers
      methodology and benchmarks but not "how to tune for your workload."
      Surface the Tier 0 config levers most users miss:
      `DurabilityPolicy::Throughput` (no fsync — correct default when the
      cloud holds the durable copy), `FlushPolicy::Manual` + `append_all`
      (amortize lock + encode + file creation across a batch),
      `compression_level(1)` (faster encode at marginal ratio cost vs the
      default 3), `for_each_from` over `read_from` for drain-side hot paths
      (~21× faster on in-memory items — see FEATURES.md). Deliver as a new
      section in `docs/PERFORMANCE.md` or a standalone `docs/TUNING.md`.

---

## See also

- [ROADMAP.md](ROADMAP.md) — long-term direction: async I/O, envelope v2
  (streaming CBOR early-stop, Blake3 checksum, compression negotiation,
  metadata block, streaming cipher), second `SegmentStore` impl.
- [CHANGELOG.md](CHANGELOG.md) — shipped work.
- [`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
  — full rationale for the envelope v2 deferrals.
