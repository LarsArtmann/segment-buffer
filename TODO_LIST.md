# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Shipped work lives in [CHANGELOG.md](CHANGELOG.md). This file tracks only
pending or in-progress work.

Status legend: `[ ]` pending · `[~]` in progress.

---

## v0.5.0 candidates (next breaking batch)

Deferred breaking changes — batch them so users upgrade once.

- [ ] **`Arc<dyn SegmentCipher>` instead of `Box`** — so `SegmentConfig` can be `Clone`. Today the `Box` makes the config non-`Clone`, which surprises callers who expect to inspect/reuse it.
- [ ] **`SegmentIter<'_, T>` lending iterator type** — return an actual GAT-based iterator from `for_each_from` instead of taking a closure, for true iterator ergonomics (`for (seq, item) in buf.iter_from(0)?`).
- [ ] **`IoSite` enum for `SegmentError::Io`** — replace `Option<PathBuf>` with `IoSite::Dir | IoSite::Segment(PathBuf) | IoSite::Unknown` to make the "no path" case explicit.
- [ ] **`TryClone` story for `SegmentConfigBuilder`** — once `.cipher(Box::new(...))` is called, the builder is non-`Clone`. Either document loudly or provide a `TryClone` that errors on cipher-bearing configs.
- [ ] **mtime probe for scan cache** — cheap `stat` to validate the cache against external directory manipulation (today the cache is invalidated only by in-process mutations).

## Concurrency & provability

- [ ] **Loom test for `delete_acked` + `append` interleaving** — requires abstracting I/O behind a trait loom can mock; real engineering work.
- [ ] **Consider `RwLock` for read-heavy workloads** — `read_from` is read-only; `append`/`flush`/`delete_acked` write. Measure first.
- [ ] **Stress test: 16 writers × 4 readers × 1M events with p50/p99 latency histogram** — today's stress test reports throughput only, not latency distribution.

## Format & storage

- [ ] **Per-segment Blake3 checksum** in the reserved envelope bytes (bit-rot detection distinct from cipher auth failures).
- [ ] **Envelope v2 design doc** — sketch the migration path for when v2 lands.
- [ ] **Compression-algorithm negotiation** via reserved byte (zstd, lz4, none).
- [ ] **Metadata block in envelope** (item count, byte count, schema hash).
- [ ] **`SegmentStore` trait** abstraction (local FS, S3, in-memory) — defer until second impl exists.
- [ ] **Async I/O feature** (tokio) — preserve "mutex never held across I/O" invariant under cancellation.
- [ ] **ChaCha20-Poly1305 cipher** under a feature flag.
- [ ] **XChaCha20-Poly1305** for extended nonces (no 2^32 message limit per key).

## Performance

Worked through in the 2026-07-20 PGO session. Outcomes below; numbers and
analysis in `docs/perf/2026-07-20_hot-path-flamegraph.md` and
`docs/perf/2026-07-20_read-from-scan-cache.md`.

- [x] **Profile-guided optimization of the hot path** — flamegraph showed 66%
      of `flush` CPU was in `__memset` from zstd re-initialising its ~200 KB `CCtx`
      on every `encode_all` call. Fixed by pooling a `zstd::bulk::Compressor` on
      `SegmentBuffer`. Result: `append/batch_1` 15.09 µs → 7.75 µs (2.07× faster),
      `batch_100` −24%, `batch_10000` −10%. See
      `docs/perf/2026-07-20_hot-path-flamegraph.md`.
- [x] **Consider `SmallVec<[T; 16]>` for `unflushed`** — **REJECTED.** A/B
      benchmarked against the post-compressor-pooling baseline: `batch_1` +3.2 %
      regression, `batch_1000` +8.5 % regression, `batch_100`/`batch_10000`
      within noise. SmallVec's spill-tracking overhead exceeds the saved initial
      allocation. No dep added. Documented in
      `docs/perf/2026-07-20_hot-path-flamegraph.md` ("What this is NOT").
- [x] **Bench `read_from` after the scan cache landed** — added
      `read_from_scan_cache` benchmark group with cold-vs-warm variants across
      10/100/1000 segments. Cache wins 6–9 % at 10 and 100 segments (the design
      regime); at 1000 segments the readdir cost is no longer dominant and the
      cold-vs-warm gap is lost in noise. Also surfaced a separate future win:
      streaming-deserialise early-stop at `limit` (today `read_segment` decodes
      the whole segment regardless of limit). See
      `docs/perf/2026-07-20_read-from-scan-cache.md`.
- [ ] **Pool the read-side zstd `DCtx`** — symmetric to the write-side
      `Compressor` pooling that landed today. `read_segment` still calls
      `zstd::decode_all` per segment, which constructs a fresh `DCtx` each time.
      Likely a similar-magnitude win on read-heavy workloads; deferred until a
      read-heavy benchmark exists to size it.
- [ ] **Streaming deserialise + early-stop at `limit`** — today `read_segment`
      CBOR-deserialises the whole segment into `Vec<T>` regardless of the
      caller's `limit`. The flat ~1.4 ms across `limit_100`/`limit_1000`/
      `limit_10000` in the bench above is the signature. A streaming decoder
      that stops after `limit` items would convert the per-call cost to
      `O(limit)` instead of `O(segment_size)`.

## Docs & polish

- [ ] **Skill-contract debt** — produce the HTML artifacts required by the `code-quality-scan`, `architecture-review`, `full-code-review`, and `nix-flake-migration` skills (or explicitly renegotiate them).

## CI / tooling

- [ ] **macOS flake verification** (`aarch64-darwin`, `x86_64-darwin`) — flake check only runs on x86_64-linux today.
- [ ] **Sign commits** — `sign-commit = true` is set in `release.toml` and `commit.gpgsign = true` in git config, but SSH signing fails: `gpg.ssh.allowedSignersFile` is not configured. Tags are signed; regular commits are not.
- [ ] **Enable auto-merge for dependabot PRs** — today dependabot PRs pile up until manually merged (8 were open during the CI-broken window). Requires `gh repo edit --enable-auto-merge` + `auto-merge: true` per updater in `dependabot.yml` + a branch-protection rule allowing auto-merge. Policy decision, not a one-liner.

## Investigation

- [ ] **Tighten `T: 'static`** — investigate whether it can be relaxed (needed for the mutex, but worth confirming).
- [ ] **Extract AES-GCM cipher into its own feature/crate boundary** for users who want only the trait.
- [ ] **Profile the hermetic Nix build** (~164s for test check; most is zstd-sys compiling bundled C). Could pre-build zstd as a Nix dependency via `ZSTD_SYS_USE_PKG_CONFIG=1`.
- [ ] **Investigate whether `include_str!("../README.md")` should be replaced** — the crate-root rustdoc embeds README.md via `include_str!`, which `craneLib.cleanCargoSource` strips from the Nix sandbox (fixed by a `postUnpack` copy in `flake.nix`, commit `b2e7c4f`). A separate `src/README.md` snippet or a hand-written crate-level doc would dodge this class of bug entirely. Low priority — the `postUnpack` fix works.
- [ ] **Consider `cargo supply-chain` crate** for downstream-auditable dependency provenance (belt-and-braces alongside `cargo deny` + `cargo audit`).

## Crates.io publishing

- [ ] **Set up a crates.io API token** in GitHub Actions secrets for automated publishing on tag (`CARGO_REGISTRY_TOKEN` — the `publish.yml` workflow is dormant without it).
