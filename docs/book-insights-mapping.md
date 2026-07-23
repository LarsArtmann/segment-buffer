# Theory-to-Practice Map: Distributed Systems Books vs segment-buffer

A grounding exercise: mapping insights from seven foundational books onto what `segment-buffer` already does, what it should adopt, and what it must never become.

**Books covered:**

- *Designing Data-Intensive Applications* (Kleppmann) — flagged IMPORTANT
- *Deciphering Data Architectures* (warehouse vs fabric vs lakehouse vs mesh)
- *Exploring CQRS and Event Sourcing*
- *Implementing DDD, CQRS and Event Sourcing*
- *Designing Event-Driven Systems* (Kleppmann)
- *Patterns of Distributed Systems* (Joshi)
- *Service Design Patterns* (Daigneau)

---

## Already Applied

### From Designing Data-Intensive Applications (Kleppmann)

| Insight | Where in segment-buffer |
|---|---|
| **The log is the central abstraction** | Segment files are an append-only log. The filename `seg_{start}_{end}.zst` encodes the offset range — the same idea as Kafka partition offsets or a WAL, just file-per-batch instead of append-to-one-file. |
| **Storage engine bifurcation (write-optimized)** | The whole design is LSM-tree philosophy: batch writes in memory (`unflushed: Vec<T>`), flush as sequential writes, compress to reduce write amplification. Correct choice for a write-heavy buffer. |
| **Durability spectrum made explicit** | `DurabilityPolicy` (`Maximal`/`Segment`/`Throughput`) is literally the PACELC tradeoff ("Else, choose Latency vs Consistency") made into a `Copy` enum. This is textbook DDIA — the tradeoff was always there, we just made it visible and configurable instead of pretending it didn't exist. |
| **At-least-once + idempotency lives in the caller** | The crate delivers at-least-once via `append → flush → read_from → delete_acked`. Server-side dedup on `(producer_id, seq)` is explicitly the consumer's concern. This is the "Idempotent Receiver" pattern done correctly — the boundary between the two responsibilities is clean. |
| **Atomic write protocol** | `tmp → sync_all → rename` in `RealStore::write_atomic` is the standard atomic-commit pattern from DDIA's reliability chapter. |
| **State derived from truth, not stored separately** | Filename-as-WAL means no metadata DB, no separate state file, no WAL. State is *derived* from the filesystem on recovery. This is the "turn the database inside-out" principle at micro-scale. |
| **Replication lag as latency problem** | The `unflushed → flush → segment` pipeline is async by design. The gap between `append` (ack) and `flush` (durability) is acknowledged and bounded by `FlushPolicy`. |
| **Quantitative scalability** | The `scaling` example (1M–100M items), criterion benchmarks, and the `store_pressure()` metric `[0.0, 1.0]` are all quantitative load descriptions, not hand-waving. |

### From Patterns of Distributed Systems (Joshi)

| Pattern | Implementation |
|---|---|
| **Write-Ahead Log** | Segment files serve the same purpose — durable record of events for crash recovery. |
| **High-Water Mark** | `head_seq` is the high-water mark: the safe-to-delete-up-to point. `delete_acked` advances it. |
| **Idempotent Receiver** | `remove_segment` is idempotent on `NotFound`. `delete_acked` clamps rather than errors on repeated acks. |
| **Single Leader / fencing** | The `flock`-based single-process invariant is a single-leader model with a fencing token (the lock fd itself). |
| **Generation Clock / fencing token** | Sequence numbers (`u64`, monotonic, gap-free) serve as fencing tokens — a consumer knows exactly which items it has processed. |

### From CQRS / DDD / Event Sourcing

| Insight | Implementation |
|---|---|
| **Write model ≠ read model** | Write path: `append → unflushed: Vec<T>`. Read path: `read_from → scan segments + drain tail`. Structurally different, optimized for different things. Not full CQRS, but the separation instinct is there. |
| **Aggregate as consistency boundary** | `SegmentBuffer<T>` is the aggregate root. The mutex enforces invariants within one aggregate. Cross-boundary coordination (cursor, cloud upload) is explicitly delegated — the AGENTS.md layer-split table is pure DDD bounded-context thinking. |
| **Domain language is load-bearing** | `docs/DOMAIN_LANGUAGE.md` exists and is well-maintained. Terms like segment, head_seq, unflushed, ack are used consistently in code, docs, and errors. |
| **Events are immutable facts** | Segment files are immutable once written. The only mutator is `delete_acked` (whole-file removal). This is event-sourcing discipline without the ceremony. |
| **Projection over the log** | `read_from(start, limit)` is a projection over the segment log. The consumer builds its own read model (the cloud sync's SQLite cursor). |

### From Designing Event-Driven Systems (Kleppmann)

| Insight | Implementation |
|---|---|
| **Source of truth vs derived data** | The segment files ARE the source of truth; in-memory state is derived (rebuilt on recovery). The consumer's cloud copy is derived from the segments. |
| **CDC pipeline** | The `read_from → upload → delete_acked` cycle IS a CDC pipeline — the buffer is the local change log, the cloud is the derived store. |

### From Deciphering Data Architectures

| Insight | Implementation |
|---|---|
| **Data as a product** | The buffer IS a data product boundary. It produces ordered, sequenced, acknowledged data for downstream consumption with a clear contract (sequence numbers, at-least-once). |

### From Service Design Patterns (Daigneau)

| Insight | Implementation |
|---|---|
| **Contract evolution management** | `#[non_exhaustive]` on every public enum/struct is semver-safe contract evolution. The envelope version byte is format-level versioning. |
| **Loose coupling via abstraction** | `SegmentStore` trait (I/O), `SegmentCipher` trait (encryption), `FlushPolicy` enum (strategy) — all enable substituting implementations without touching the orchestration layer. |

---

## Should Apply (Gaps Worth Closing)

### 1. Document the consistency model as a contract (DDIA)

**Gap:** The at-least-once guarantee is documented, but the *session guarantees* the caller implicitly gets are not spelled out.

**Recommendation:** Add a "Consistency Model" section to the rustdoc and DOMAIN_LANGUAGE.md that explicitly states:

- **Read-your-writes:** Does a `read_from` after a successful `append` always see the appended item? (Yes — same lock, `next_seq` is bumped before the lock releases.)
- **Monotonic reads:** Does a consumer reading at increasing `start` offsets always see forward progress? (Yes — segments are immutable and append-only.)
- **Consistent-prefix:** Does `read_from` return a contiguous prefix, or can it skip? (It returns whatever is on disk + in memory, contiguous by construction.)

This is a 30-minute documentation task, not a code change, and it directly maps to DDIA's session-guarantees framework.

### 2. Percentile latency targets, not just throughput (DDIA)

**Gap:** The criterion benchmarks measure throughput and mean latency. DDIA's central lesson is that **percentiles (p99, p99.9) reveal tail latency that averages hide**.

**Recommendation:** The `hotpath_profile` example is a start. Add p99/p99.9 reporting to the bench summaries (criterion already produces this data — it just needs to be called out in the perf docs). Document the expected p99 target for `append` (the hot path) and verify it's not regressing across releases.

### 3. Explicit saga documentation (Patterns of Distributed Systems)

**Gap:** The `read_from → upload → delete_acked` cycle is a saga with compensating actions (retry on failure), but it's not documented using that vocabulary.

**Recommendation:** In `examples/cloud_sync.rs` and `examples/cloud_sync_disk_full.rs`, add a header comment that names this as a **saga pattern** and identifies the compensating action (the disk-full backoff). This helps consumers recognize the pattern and reason about failure modes. Low effort, high clarity payoff.

### 4. Health check primitive (Patterns of Distributed Systems)

**Gap:** There's no `health()` method. A consumer opening a buffer has no way to verify the directory is writable, the lock is still held, and the filesystem isn't full, short of trying an operation and catching the error.

**Recommendation:** A `fn health(&self) -> Result<HealthReport>` that probes:

- Directory writability (write + delete a sentinel file)
- Lock file validity (still holds the flock)
- `store_pressure()` snapshot
- Free disk space (if queryably

This is a small, well-scoped addition that maps to the Heartbeat pattern and improves operability (one of DDIA's three pillars). Consider whether this is v0.6.0 material or TODO_LIST-worthy.

### 5. Streaming AEAD for large segments (DDIA + cipher evolution roadmap)

**Gap:** The whole segment is buffered (CBOR → zstd → encrypt as a blob). For large batches, this bounds memory at `segment_size`. The AGENTS.md already identifies this as a v0.6+ direction.

**Recommendation:** Keep this on the roadmap as-is. A streaming AEAD (RFC 8450 chunked format or libsodium's `crypto_secretstream`) would bound memory and enable early-stop-at-`limit` reads. This is the most architecturally significant evolution item and correctly deferred.

### 6. CBOR schema versioning documentation (CQRS/ES)

**Gap:** The envelope has a version byte, but the CBOR payload inside (the `T` serialization) is unversioned. For a generic `T: Serialize`, schema evolution is the caller's problem — but this isn't documented.

**Recommendation:** Add a section to DOMAIN_LANGUAGE.md or a dedicated `docs/SCHEMA_EVOLUTION.md` that explains: (a) the envelope handles format-level versioning, (b) CBOR schema evolution of `T` is the caller's concern, (c) if `T` changes in a backward-incompatible way, the caller needs a migration strategy (upcasters, versioned types, etc.). This maps directly to the "event versioning is brutal" lesson from the CQRS/ES books.

---

## Should NOT Do (Anti-Patterns for This Crate)

### From Designing Data-Intensive Applications

| Do NOT | Why |
|---|---|
| **Multi-leader or leaderless replication** | Single-process by design. The `flock` enforces one owner. Adding multi-process support would reintroduce every distributed-system problem (split-brain, consensus, double-delivery) that this crate deliberately avoids. |
| **Consensus protocols (Raft/Paxos)** | There is no multi-node coordination. The directory is a single copy. Consensus would be pure overhead for zero benefit. |
| **2PC across buffer + consumer** | The ack model is simpler and sufficient. 2PC adds a blocking coordinator failure mode for no gain. |
| **Serializable isolation across reads/writes** | The crate provides at-least-once, not linearizable reads. Adding a `Serializable` mode would be scope creep — if the consumer needs serializable, they handle it server-side. |

### From Deciphering Data Architectures

| Do NOT | Why |
|---|---|
| **Data lakehouse / columnar storage (Parquet/Iceberg)** | Segment files are optimized for sequential append + read, not analytical scans. Adding columnar would add complexity for a use case that doesn't exist here. |
| **Data mesh / federated governance** | These are organizational patterns for enterprise data platforms. A single library cannot be a mesh. |
| **Data fabric / metadata layer** | The filename-as-metadata approach is deliberately simpler. Adding a metadata catalog would reintroduce the exact state-management complexity the crate avoids. |

### From CQRS / Event Sourcing

| Do NOT | Why |
|---|---|
| **Full event sourcing framework (upcasters, snapshots, projection manager)** | The crate stores generic `T`, not domain events. Adding ES infrastructure would be massive scope creep. The consumer (monitor365) owns the event model. |
| **Multiple read-model projections inside the crate** | The crate provides `read_from` / `for_each_from`. Building a projection manager would pull the consumer's concerns (which read models? what shape?) into the buffer. |
| **Event store semantics (replay, time-travel queries)** | Segments are deleted after ack. They are not an infinite event log. Adding "replay from time T" would require never deleting, which breaks the bounded-queue contract. |
| **Aggregate root hierarchy / domain event bus** | This is a single-purpose library, not a domain model. DDD aggregate ceremony would add indirection for no domain insight. |

### From Patterns of Distributed Systems

| Do NOT | Why |
|---|---|
| **Leader election** | One owner process, enforced by `flock`. No cluster to elect from. |
| **Quorum reads/writes** | Single copy. N=1, so R=1, W=1. Quorum logic is meaningless. |
| **Gossip protocol** | Single process. No cluster state to disseminate. |
| **Request pipeline / async messaging API** | The synchronous API is a **feature** — "no hidden threads" is part of the crate's identity. Adding an async runtime or internal message bus would violate this. |

### From Service Design Patterns

| Do NOT | Why |
|---|---|
| **Async/callback API (`Request/Acknowledge/Notify`)** | The sync API is the contract. The caller controls threading and timing. An async API would impose a runtime choice (tokio? async-std? smol?) on every consumer. |
| **Service versioning with multi-version coexistence** | Semver handles this at the crate level. Building in-request version negotiation is over-engineering for a library. |
| **Discovery / registry** | The crate is `open(dir, config)`. No service registry needed. |

---

## The one-sentence summary

segment-buffer already applies the **log-as-truth**, **write-optimized storage**, **explicit durability tradeoffs**, **at-least-once + caller-side idempotency**, **aggregate-as-consistency-boundary**, and **domain-language-first** principles from these books. The real gaps are documentation (consistency model contract, schema evolution, saga vocabulary) and one small feature (health check) — not architecture. The biggest trap to avoid is pulling distributed-systems patterns (consensus, replication, multi-process coordination) into what is deliberately and correctly a single-process local buffer.
