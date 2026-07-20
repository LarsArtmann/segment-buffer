# Envelope v2 — design doc and migration path

**Status:** DESIGN — pending a concrete consumer request that forces the bump.
**Scope:** Sketches the migration path for when v2 lands; folds the v1
reserved-bytes limitations (M17 Blake3 checksum, compression-algorithm
negotiation, metadata block) into a single coherent plan so they do not
get designed in isolation.

---

## Why v2 is not landing yet

The v1 envelope (8 bytes: `SBF1` magic + 1 version byte + 3 reserved bytes)
is intentionally minimal. It exists to make legacy detection tractable
(false-positive rate 2⁻⁵⁶ on random-bytes inputs) while leaving room for
future evolution. Today:

- The 3 reserved bytes are too small for a useful checksum (24-bit CRC is
  too weak at the 597M-segment monitor365 scale).
- The cipher is implicitly "whatever the buffer was opened with" — there
  is no per-file marker distinguishing AES-GCM from XChaCha20-Poly1305.
  A buffer opened with the wrong cipher fails on first read with an
  opaque AEAD tag-mismatch, not a clear "this file was written with a
  different cipher."
- The compression algorithm is fixed at zstd. There is no per-file marker
  for LZ4, Snappy, or "no compression" (which would matter for read-heavy
  workloads on hosts without a fast zstd decoder).

These are real limitations. They are not yet painful enough to justify the
cost of a v2 bump (format change + migration tooling + byte-compat surface
area). v2 lands when **one of them becomes painful enough to ship**:

- A bit-rot detection incident (silent corruption that an integrity check
  would have caught).
- A "wrong cipher" misconfiguration that operator-time cannot diagnose.
- A concrete read-heavy workload where LZ4 (or no compression) is a
  measurable win over zstd.

Until then, v1's strict-additive detection (auto-fallback to legacy) keeps
the migration cost low.

---

## v2 layout (proposed)

```
offset  bytes   meaning
------  -----   -------
  0..4   4      magic: ASCII "SBF1"            (unchanged from v1)
   4     1      envelope version = 2           (bumped)
   5     1      cipher id (0=none, 1=AES-256-GCM, 2=XChaCha20-Poly1305, 0xFF=unknown)
   6     1      compression id (0=zstd, 1=lz4, 2=snappy, 3=none, 0xFF=unknown)
   7     1      checksum id (0=none, 1=CRC32C-truncated, 2=Blake3-64bit-truncated, 0xFF=unknown)
  8..4   4      item count (u32 LE)            — supports early-stop deserialise
 12..8   8      uncompressed payload byte count (u64 LE) — supports exact-capacity decompress
 20..   N       payload (the v2 bytes: compression(CBOR(events)), optionally encrypted)
 20+N.. 8       checksum over [offset 0 .. 20+N] (Blake3-64 or CRC32C depending on id)
```

Total header: 20 bytes (vs v1's 8). The growth is the cost of the
new metadata. The trailing checksum is **outside** the encrypted payload
so it can be verified before decryption (early-tamper detection without
the cipher's AEAD tag, useful for bit-rot distinct from malicious
tampering).

### Reserved-byte repurpose rule (breaking)

v1 reserved bytes (offset 5..8) MUST be zero for the v1-detection
invariant. v2 repurposes them — a v2 file is detected by `version == 2`,
NOT by the reserved-byte-zero invariant. The v1 detector in
`unwrap_envelope` continues to fall back to "treat as legacy v1" when
the version is anything other than 1 AND the magic matches AND reserved
bytes are zero. v2 files will NOT match the v1 detector (because the
reserved bytes are nonzero), so:

- A v1 reader sees a v2 file as "no envelope, treat as legacy v1" → it
  attempts plaintext decode and fails with a clear CBOR/zstd error. **v2
  readers MUST reject v1 files** with a clear error to prevent the
  opposite confusion.

### Cipher auto-detection vs explicit id

The cipher id at offset 5 lets a v2 reader pick the right cipher without
the buffer having been opened with that cipher configured. This is the
"wrong cipher misconfiguration" fix. Implementation: the buffer's
configured cipher is used to **write**; on **read**, the id selects which
cipher impl to invoke. AES-GCM and XChaCha20-Poly1305 are both available
under the `encryption` feature; the id routes the bytes.

### Compression negotiation

The compression id at offset 6 lets v2 readers decompress without guessing.
Today's v1 always-zstd assumption becomes a v1 reader's invariant; v2
files can carry lz4/snappy/none per-file. Negotiation is per-write — the
buffer's compression level becomes a per-buffer default, but a future API
(`append_with_compression(item, Compression::Lz4)`) can override per
batch for hot-path items where zstd's encode cost matters.

### Early-stop deserialise (M14)

The item-count field at offset 8..12 lets a v2 reader skip the streaming
deserialise complexity (M14) by sizing the output Vec exactly — no
heuristic capacity, no retry-on-too-small. Combined with the
uncompressed-payload-size field at offset 12..20, the decompressor can
also size its output buffer exactly. This is the path that retires M14's
ciborium-API blocker: the format itself encodes the bounds the current
API can't read.

### Bit-rot checksum (M17)

The trailing checksum (offset 20+N .. 20+N+8) is a Blake3-64-truncated or
CRC32C over the entire envelope. This is **distinct from the cipher's
AEAD tag**:

- AEAD tag detects modification by an attacker who lacks the key.
- The trailing checksum detects bit-rot by the storage layer (disk
  corruption, controller fault, RAID rebuild bug) on ciphertext. The
  ciphertext is what's on disk; AEAD would also catch bit-rot, but
  only if the cipher is configured (a plaintext buffer has no AEAD).

The checksum catches bit-rot on plaintext buffers — the gap M17 fills.

---

## Migration path (when v2 lands)

1. **Write side:** the buffer's `write_atomic` learns to emit v2 bytes
   when configured (a new `SegmentConfig.envelope_version` knob, default
   1 for one release).
2. **Read side:** `unwrap_envelope` gets a `Version::Two` branch that
   parses the new header fields and dispatches on the cipher/compression
   ids. `Version::One` (and the legacy fallback) keep working unchanged.
3. **Mixed directories:** v1 and v2 segments can coexist in the same
   directory during the rollout — recovery's filename scan is byte-format
   agnostic (it only parses filenames).
4. **End-of-life for v1:** one release after v2 ships, default flips to
   v2 for new writes. v1 reads stay supported indefinitely (legacy byte
   compat with monitor365 is a hard constraint).
5. **No automatic rewrite.** There is no "upgrade segments in place"
   tool — segments are short-lived (drained to the cloud and deleted
   within hours). Long-lived on-disk data is the cloud's concern, not
   the buffer's.

---

## Deferred to v2 (the M14/M17/etc. closure)

This doc supersedes the standalone TODO entries for:

- **M14 (streaming CBOR deserialise + early-stop at limit):** deferred.
  The ciborium public API does not expose the streaming Deserializer
  struct, and the format itself encodes no item count, so the clean
  early-stop path requires either forking ciborium or changing the
  envelope. v2's item-count field is the format change that retires
  M14; until then, the read path pays O(segment_size) regardless of
  limit (measured at ~1.4 ms per segment — not on the cloud-sync hot
  path because segments are small and the read+drain loop is bound by
  the cloud endpoint, not the local decode).
- **M17 (per-segment Blake3 checksum):** deferred. v1's 3 reserved
  bytes are too small for a useful checksum (24-bit collision is too
  weak at scale). v2's trailing-checksum design is the path; without
  v2, the cipher's AEAD tag covers authenticated corruption (the
  common case), and bit-rot on plaintext buffers is the residual gap.
- **Compression-algorithm negotiation:** deferred. zstd is good enough
  for the cloud-sync use case (the read side is rarely the bottleneck);
  LZ4's decode speed advantage matters only for read-heavy workloads
  that segment-buffer is not the bottleneck for. v2's compression-id
  field is the path when a concrete workload forces it.
- **Metadata block (item count, byte count, schema hash):** folded
  into v2's header directly (offset 8..20). No separate design needed.
- **Async I/O feature:** deferred independently. Preserving the
  "mutex never held across I/O" invariant under async cancellation is
  a large design surface ( cancellation safety of in-flight flush,
  what happens to `head_seq` if the future is dropped between
  `take(unflushed)` and `write_atomic`, etc.). No current consumer
  needs async — the drain loop is naturally synchronous, and
  async-to-sync bridges work fine when segment-buffer is the leaf.
- **`SegmentStore` trait abstraction (S3, in-memory, second impl):**
  deferred until the second concrete consumer exists. The trait is
  already shipped (`src/store.rs`) and used by loom tests; adding a
  second production impl (S3-backed) without a real S3 consumer would
  be speculative. When a real second impl lands, the trait surface will
  shape itself to that consumer's actual needs (streaming reads?
  partial writes? range scans?).
- **Streaming/incremental cipher (RFC 8450 chunked AEAD):** deferred
  to v0.6+. The whole-segment buffer is fine for the cloud-sync use
  case (segments are bounded by `Batch(N)`); a streaming cipher would
  bound memory on huge segments and enable early-stop-at-`limit` reads
  of encrypted data. Neither is a current pain point.

These items are NOT "we'll never do them." They are "v2 is the right
place, and v2 is not landing until one of them becomes painful." The
list above is the migration-path backlog.
