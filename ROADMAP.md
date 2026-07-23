# Roadmap

Long-term direction and raw ideas, not yet refined into actionable work.
Near-term, bounded tasks live in [TODO_LIST.md](TODO_LIST.md); shipped work
lives in [CHANGELOG.md](CHANGELOG.md); current capabilities in
[FEATURES.md](FEATURES.md). This file holds only what is **not yet built**.

The design priorities that shipped (correctness, durability, minimal surface,
synchronous, no hidden threads, no WAL, no async runtime) are settled.
Everything below should preserve those properties unless it explicitly says
otherwise.

---

## Direction

### Async I/O (optional)

All file I/O is synchronous today; the mutex is never held across await points.
An optional async API (`tokio` / `async-std` feature) would let callers
integrate `SegmentBuffer` into async pipelines without offloading every call
to `spawn_blocking`. The hard part is preserving the "mutex never held across
I/O" invariant under cancellation — a large design surface with no current
consumer.

### Streaming / incremental cipher

Today the whole segment is buffered (CBOR → zstd → encrypt as a blob). A
streaming AEAD (e.g. RFC 8450 chunked format) would bound memory on large
segments and enable early-stop-at-`limit` reads of encrypted data. This is a
format change and is tracked under envelope v2. All cipher impls must stay
self-describing (nonce in-band) to honor the trait contract.

### Second `SegmentStore` impl

The `SegmentStore` trait already shipped (`src/store.rs`, reachable externally
only under the `loom` feature). A second production impl (S3-backed,
encrypted-block-device, etc.) is deferred until a concrete consumer exists —
adding one without a real consumer would be speculative. When a real consumer
lands, the trait surface will shape itself to that consumer's actual needs
(streaming reads? partial writes? range scans?). The filename contract
remains the recovery source of truth regardless of which impl backs the
buffer.

### Fuzzing & integrity — optional Blake3 checksum

A `cargo-fuzz` scaffold, CI integration, and proptest analogues all shipped
(see FEATURES.md). The remaining gap is an optional per-segment checksum
(e.g. Blake3) for detecting bit-rot distinct from cipher authentication
failures. This is a format change and is tracked under envelope v2.

### Observability — richer metrics

`stats()` (single-lock snapshot), `RecoveryReport`, `sync_disk_bytes`, and the
pooled zstd contexts all shipped (see FEATURES.md). Future: richer
per-segment metrics (segment count, per-segment size histogram).

### Envelope v2 / format change

Long-term format change. The v2 design ships a 20-byte header (cipher id,
compression id, checksum id, item count, uncompressed size) plus a trailing
Blake3 / CRC32C checksum, and unlocks:

- Streaming CBOR deserialise with early-stop at `limit` (the item-count field
  retires the current O(segment_size) read cost regardless of `limit`).
- Per-segment checksum (bit-rot detection on plaintext buffers, distinct from
  the cipher's AEAD tag).
- Compression-algorithm negotiation (per-file zstd / lz4 / snappy / none).
- Metadata block (item count, uncompressed byte count — supports exact-capacity
  decompress).
- Cipher auto-detection (cipher id byte — the "wrong cipher misconfiguration"
  fix).

Will not land until one of those features becomes painful. See
[`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`](docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md)
for the full layout, migration path, and trigger conditions.

---

## Non-goals (by design)

- **No WAL.** The filename IS the WAL. Adding one would double the durability
  story and the write amplification.
- **No embedded database.** If you need SQLite-grade querying, use SQLite. This
  crate optimizes for append/read/ack throughput, not ad-hoc queries.
- **No built-in admission policy.** `store_pressure()` is the signal; the
  policy is yours (see `examples/backpressure.rs`).
- **No built-in background flush worker.** The decoupling is a caller-owned
  `FlushPolicy::Manual` + timer thread — see
  `examples/background_flush.rs`. A library worker would break the
  synchronous-no-hidden-threads identity, worsen error propagation, and
  duplicate what `FlushPolicy::Manual` + a user timer already achieves. See
  `docs/planning/2026-07-21_08-26_flush-worker-and-tier-0-levers.md` §
  "Addendum" for the full rationale.

---

## Reference analyses

- [`docs/book-insights-mapping.md`](docs/book-insights-mapping.md) — maps
  seven distributed-systems books (DDIA, CQRS/ES, Patterns of Distributed
  Systems, etc.) against this codebase: what is already applied, what should
  be applied, and what anti-patterns to avoid. The action items from that
  analysis (consistency model docs, schema evolution docs, tradeoffs matrix,
  allocation-count guard) have been executed; the mapping doc is retained as
  the rationale record.
- [`docs/planning/2026-07-23_15-50_book-insights-action-plan.md`](docs/planning/2026-07-23_15-50_book-insights-action-plan.md)
  — the Pareto execution plan for closing the documentation and design gaps
  identified by the mapping.
