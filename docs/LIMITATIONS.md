# Limitations

What segment-buffer does **not** do, and why. Every limitation here is a
deliberate design decision or an accepted tradeoff, not an oversight. If a
limitation says "by design", it means changing it would break a core invariant
or re-introduce a problem the crate was created to solve.

> See also: [DOMAIN_LANGUAGE.md](./DOMAIN_LANGUAGE.md) for the consistency
> model and tradeoff matrix; [ROADMAP.md](../ROADMAP.md) for non-goals and
> future direction.

---

## Process model

### Single-process only

One owner process per buffer directory, enforced by an exclusive `flock` on a
`.segment-buffer.lock` sidecar (since v0.5.0). A second process calling `open`
on the same directory gets `SegmentError::Locked` immediately — no block, no
timeout.

Multiple **threads** inside the owner process are fully supported (MPMC via
`parking_lot::Mutex`). Multiple **processes** are not. If you need
multi-process access, use a single owner process and expose it via IPC (Unix
socket, HTTP, etc.).

**Why:** multiple writers would race on segment filenames, double-deliver
items, and corrupt `head_seq` / `next_seq`. The lock is the crate's identity
boundary.

### Synchronous API only

All I/O is synchronous. There are no `async fn` methods and no hidden threads.
The mutex is never held across file I/O.

**Why:** an async API would need to preserve the "mutex never held across I/O"
invariant under cancellation — a large design surface with no current consumer.
See [ROADMAP.md](../ROADMAP.md) for the async direction.

### No built-in background flush worker

The crate does not spawn a flush thread. `FlushPolicy::Batch(N)` runs the
encode pipeline inline on the threshold-crossing `append()`. For p99-sensitive
producers, decouple flush timing with `FlushPolicy::Manual` + a caller-owned
timer thread — see `examples/background_flush.rs`.

**Why:** a library-internal worker would break the synchronous-no-hidden-threads
identity, make error propagation strictly worse (sticky errors on next call vs
immediate), and duplicate what `FlushPolicy::Manual` + a user timer already
achieves.

---

## Delivery semantics

### At-least-once, not exactly-once

The crate guarantees **at-least-once** delivery. Between `read_from(start, ...)`
and `delete_acked(start + count - 1)`, a crash leaves the batch on disk. On
restart, `read_from(start, ...)` returns it again. Making this **effectively
once** requires server-side idempotency on `(producer_id, seq)` — see
`examples/idempotent_server.rs`.

### No cursor persistence

The crate does not own a cursor file. After a crash, the caller's cursor is
lost unless the caller persists it independently. The starting cursor after
recovery is `buf.stats().head_sequence`.

**Why:** cursor persistence is the consumer's concern (monitor365 stores it in
SQLite with its own fsync discipline). Pulling it into segment-buffer would
tangle two durability models, re-introduce the per-ack fsync cost that
`Throughput` mode removes, and mis-model the per-device vs per-directory
cardinality. See the layer-split table in [AGENTS.md](../AGENTS.md).

---

## Durability

### Unflushed items are volatile

Items in the in-memory `unflushed` buffer are lost on crash. The crate does not
fsync the in-memory tail. Call `flush()` at crash-sensitive boundaries or use a
`FlushPolicy` that auto-flushes frequently enough for your durability target.

### DurabilityPolicy tradeoffs

| Policy       | Worst-case crash loss                            |
| ------------ | ------------------------------------------------ |
| `Maximal`    | last in-flight flush only                        |
| `Segment`    | rename window (~5-30s of flushes on ext4/xfs)   |
| `Throughput` | entire OS dirty window (~30s)                    |

`Segment` (today's default) fsyncs the segment file's data but NOT the directory
inode after rename. A host crash within the kernel's dir-inode flush window can
leave the renamed file's data on disk but unreachable through the directory.

`Throughput` skips all fsync entirely. This is correct when the cloud endpoint
holds the durable copy and the local disk is a throughput buffer.

**Not a bug:** the framing is not "weaken durability for speed" — it is "make
the tradeoff explicit and configurable." `Segment` was always this way; the
enum just makes it visible.

---

## Read consistency under concurrency

The canonical usage pattern — one consumer thread running `read_from` -> upload
-> `delete_acked`, sequential, no overlap — never hits any of these limitations.
They only manifest under concurrent readers, background flushers, or parallel
consumers.

### Spurious Io errors under concurrent `delete_acked`

`read_from` scans the directory unlocked, then reads each segment file unlocked.
If `delete_acked` removes a segment between scan and read, `read_from` returns
`SegmentError::Io(NotFound)`. The segment was already acknowledged — the error
is spurious. Retry the read; the next scan reflects the deletion.

**Why not fixed:** making `read_bytes` swallow `NotFound` would mask genuine
corruption (a missing segment that was NOT acked).

### Transient gaps under concurrent `flush`

`read_from`'s Phase 1 (directory scan) and Phase 2 (read `unflushed` under lock)
are separated by an unlocked gap. If `flush()` completes during that gap, items
move from `unflushed` to a segment file the scan already missed. `read_from`
returns an incomplete result for that call. The items are durable on disk; a
subsequent `read_from` observes them.

**Why not fixed:** holding the mutex across the scan-to-read gap would serialize
I/O and break the "never held across file I/O" invariant.

### No transactional multi-segment reads

`read_from` is not atomic across segments. Each segment is read individually; a
failure on segment N aborts the read (returns `Err`) without returning segments
0..N-1.

---

## Data model

### No schema evolution support for `T`

The CBOR payload inside the envelope is `serde`'s serialization of your `T`.
This layer is completely unversioned. If you change `T` in a
backward-incompatible way, old segment files will fail to decode. The crate has
no way to detect or migrate this — it does not know the shape of `T`.

See [DOMAIN_LANGUAGE.md](./DOMAIN_LANGUAGE.md) "Schema evolution of T" for
strategies (versioned enum, upcaster in drain loop, fresh buffer).

### No multi-`T` coexistence

Segments from multiple `T` versions cannot coexist in the same directory with
auto-detection. This requires envelope v2 (cipher id + metadata block), which
is on [ROADMAP.md](../ROADMAP.md) and will not land until a concrete consumer
requires it.

---

## Scope boundaries (what lives upstream)

The crate is the **producer-side local buffer**. Everything cloud-facing lives
in the consumer. This is a verified design boundary, not a TODO.

| Concern                                   | Owner      | Not in this crate because...                                    |
| ----------------------------------------- | ---------- | --------------------------------------------------------------- |
| Cloud client (HTTP, retry, auth)          | Consumer   | The drain loop is the consumer's `read_from` -> upload -> `delete_acked` cycle. |
| Cursor persistence                        | Consumer   | Tangles two durability models; mis-models per-device cardinality. |
| Backpressure / admission policy           | Consumer   | `store_pressure()` is the signal; the policy is yours. See `examples/backpressure.rs`. |
| Server-side idempotency (`event_id` dedup) | Consumer's server | The library delivers at-least-once; the server makes it effectively-once. |
| Cloud sync orchestration loop             | Consumer   | monitor365 owns this in `cli/src/cloud_sync.rs`.               |

---

## Format

### No streaming cipher (whole segment buffered)

The entire segment is buffered in memory during encode (CBOR -> zstd -> encrypt
as a blob) and decode (decrypt -> decompress -> deserialize as a blob). A
streaming AEAD (e.g. RFC 8450 chunked format) would bound memory on large
segments and enable early-stop-at-`limit` reads of encrypted data. This is a
format change tracked under envelope v2.

### No cipher auto-detection

The cipher type is not stored in the segment envelope. If you open a buffer with
the wrong cipher, encrypted segments will fail authentication (GCM/Poly1305 tag
mismatch) or produce garbage. The crate cannot tell you which cipher was used.
Cipher auto-detection (cipher id byte in the envelope) is an envelope v2
feature.

### No per-segment checksum

There is no standalone integrity checksum on segment files. Bit-rot detection
relies on the cipher's AEAD tag (if encryption is enabled) or on CBOR decode
failure (if not). A standalone Blake3 checksum would catch bit-rot on plaintext
buffers distinct from cipher authentication failures. This is an envelope v2
feature.

### No compression negotiation

Segments are always zstd-compressed. There is no per-file compression algorithm
selection (lz4, snappy, none). Compression negotiation is an envelope v2
feature.

---

## Operational

### No built-in health check

There is no `health()` method. The canonical health check is `stats()` for
pressure plus a trial `append()` + explicit `flush()` to probe writability.
Three candidate designs (`health()` wrapping `stats()`, sentinel-file write,
statfs/GetDiskFreeSpace) were rejected as Verschlimmbessern — redundant,
disk-harmful on a near-full filesystem, or a platform dependency for something
`store_pressure()` already approximates.

### No metrics export

The crate exposes `BufferStats` (a snapshot struct) and `store_pressure()` (a
ratio). It does not export Prometheus metrics, OpenTelemetry spans, or any
other observability signal. Wiring those is the consumer's job.

### No monitoring hooks

There are no callback hooks for monitoring flush events, segment writes, or
deletion events. The `tracing` crate is used internally for debug-level spans,
but there is no structured event stream for external monitoring.

---

## What these limitations enable

Every limitation above exists to preserve one or more of these properties:

| Property                 | Which limitations serve it                                      |
| ------------------------ | --------------------------------------------------------------- |
| **Synchronous, no hidden threads** | No background flush worker, no async API, no monitoring hooks |
| **Filename-based recovery** | No WAL, no metadata database, no cursor file                  |
| **Single-process ownership** | No cross-process coordination, flock-based lock               |
| **At-least-once delivery** | No exactly-once guarantee, idempotency in caller's server     |
| **Minimal dependency surface** | No cloud client, no embedded database, no metrics framework |
| **Panic-free public API** | No re-entrancy guard, documented race windows (retry, not fix) |
