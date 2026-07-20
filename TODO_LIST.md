# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Shipped work lives in [CHANGELOG.md](CHANGELOG.md). This file tracks only
pending or in-progress work.

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (recent entries
stay until the next CHANGELOG cut, then move out).

---

## v0.6+ / envelope v2

All items below are deferred pending the envelope v2 format change or a
concrete consumer request that forces the surface area. See
`docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`
for the full migration path and rationale.

- [ ] **Streaming CBOR deserialise + early-stop at `limit`** — blocked by
      ciborium's private `Deserializer` struct. The format itself encodes
      no item count; the clean early-stop path requires either forking
      ciborium or changing the envelope. v0.6's envelope v2 includes an
      item-count field that retires this.
- [ ] **Per-segment Blake3 checksum** — v1's 3 reserved bytes are too
      small for a useful checksum at scale. v0.6's envelope v2 ships a
      trailing-checksum design that covers this.
- [ ] **Compression-algorithm negotiation** — deferred to v2 (v2's
      compression-id byte is the path).
- [ ] **Metadata block in envelope** — folded into v2's header
      (offset 8..20).
- [ ] **Streaming/incremental cipher** — a streaming AEAD (e.g. RFC 8450
      chunked format) would bound memory on large segments and enable
      early-stop-at-`limit` reads. Cost: format change. Blocked on
      envelope v2.
- [ ] **`SegmentStore` second impl** (S3, in-memory, encrypted-block-device)
      — the trait shipped in v0.5.0; adding a second production impl
      without a real consumer would be speculative.
- [ ] **Async I/O feature** (tokio) — deferred to v0.6+. Preserving the
      "mutex never held across I/O" invariant under cancellation is a
      large design surface with no current consumer.
