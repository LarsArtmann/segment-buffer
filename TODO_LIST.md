# TODO List

Short- and mid-term improvement tasks — actionable, bounded, with status.
Long-term vision and raw ideas live in [ROADMAP.md](ROADMAP.md).

Shipped work lives in [CHANGELOG.md](CHANGELOG.md). This file tracks only
pending or in-progress work.

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done (recent entries
stay until the next CHANGELOG cut, then move out).

---

## v0.5.0 batch — SHIPPED 2026-07-20 (pending release tag)

All v0.5.0 candidates are done. See `CHANGELOG.md` `[Unreleased]` for the
full per-item detail. The batch implements the 2026-07-20 reframing
(single-process throughput buffer for cloud sync; configurable durability;
XChaCha20 recommended cipher; at-least-once delivery).

- [x] **`flock`-based single-process lock** (`fs4::FileExt::try_lock` on
      `.segment-buffer.lock`). Released by explicit `Drop` impl.
- [x] **`DurabilityPolicy` enum** (`Maximal` / `Segment` / `Throughput`)
      threaded through `SegmentStore::write_atomic`. `Maximal` adds
      `dir.sync_all()` after rename (closes the rename-window gap); `Throughput`
      drops the per-flush fsync. Default stays `Segment` for one release.
- [x] **`XChaCha20Poly1305Cipher`** under `encryption` (alongside legacy
      `AesGcmCipher`). `SegmentConfigBuilder::recommended_cipher(key)` installs
      it for new buffers.
- [x] **`Arc<dyn SegmentCipher>` instead of `Box`** — `SegmentConfig` and
      `SegmentConfigBuilder` are now `Clone` (the cipher `Arc` is shared
      between clones).
- [x] **`SegmentIter<'_, T>`** — `iter_from(start, limit)` returns an
      owned-item iterator yielding `(seq, item)` pairs. `for_each_from` stays
      for the lending (in-memory zero-copy) path.
- [x] **`IoSite` enum** for `SegmentError::Io` (`Dir` / `Segment(PathBuf)`
      / `Unknown`). `with_path` and the new `with_dir` tag Unknown sites at
      high-value call sites.
- [x] **`TryClone` story for builder** — became `#[derive(Clone)]` once
      the cipher moved to `Arc` (M5). No separate `TryClone` needed.
- [x] **mtime probe for scan cache** — capability-probed at `open()`
      (sentinel file + 15ms sleep + re-stat). On capable fs, `scan_segments`
      invalidates the cache when the directory mtime moves (external
      manipulation detection). On mtime-pinned fs, falls back to today's
      behavior verbatim (no `0 == 0` false-positive).

## Cloud sync & delivery

- [x] **`examples/cloud_sync.rs`** — runnable at-least-once drain loop
      with `ReliableUploader` (happy path) and `FlakyUploader` (transient
      failure + retry). Demonstrates the `head_sequence → read_from → upload
→ delete_acked` cycle.
- [x] **`examples/idempotent_server.rs`** — in-process server stub
      showing the `(producer_id, seq)` dedup pattern that the at-least-once
      model requires on the consumer side. Teaches what the library cannot
      enforce.
- [x] **Cursor file — REJECTED.** See `AGENTS.md` § "Layer split vs
      monitor365" for the rationale (cursor is per-device; mixing its fsync
      into the buffer's flush path tangles durability models).
- [x] **`Throughput` mode benchmark** — `bench_durability_policy` A/B/C's
      `Maximal` vs `Segment` vs `Throughput` on a 1000-event flush. Result on
      the dev host (nvme + ext4): `Throughput` ~142µs, `Segment` ~161µs,
      `Maximal` ~192µs. `Throughput` is ~12% faster than `Segment` and ~26%
      faster than `Maximal`.
- [x] **Disk-full backpressure example** —
      `examples/cloud_sync_disk_full.rs` demonstrates the metrics-not-policy
      pattern: producer applies backpressure via `store_pressure() >
threshold`, NEVER evicts unacked segments (at-least-once hard no).
- [ ] **Streaming/incremental cipher** — deferred to v0.6+. See
      `docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`.

## Concurrency & provability

- [x] **Loom test for `delete_acked` + `append` interleaving** — DONE
      2026-07-20 (SegmentStore trait extraction + MockStore).
- [x] **Consider `RwLock` for read-heavy workloads** — INVESTIGATED,
      Mutex kept. The 16-writer × 4-reader latency stress test (M16) shows
      p99 under 50ms in debug CI under heavy contention — Mutex's per-op
      overhead advantage beats RwLock's read-scaling for the cloud-sync
      drain workload (1 writer + 1 reader typical). Revisit if a real
      read-heavy multi-reader workload exhibits Mutex contention.
- [x] **Latency stress test with p50/p99 histogram** —
      `stress_8_writers_4_readers_latency_histogram` reports p50/p90/p99/p99.9
      per-append latency. p99 soft guard at 50ms (debug-mode CI).

## Format & storage

- [ ] **Per-segment Blake3 checksum** — DEFERRED to v0.6 (v1's 3 reserved
      bytes are too small for a useful checksum at scale; v2's trailing
      checksum design is the path).
- [x] **Envelope v2 design doc** —
      `docs/planning/2026-07-20_05-50_envelope-v2-design-and-v0.6-deferrals.md`.
      Sketches v2 layout, the migration path, and folds M14/M17/etc. deferral
      rationale into a single document.
- [ ] **Compression-algorithm negotiation** — DEFERRED to v2 (v2's
      compression-id byte is the path).
- [ ] **Metadata block in envelope** — folded into v2's header
      (offset 8..20).
- [ ] **`SegmentStore` trait** abstraction (local FS, S3, in-memory) —
      DEFERRED until second impl exists. The trait is already shipped; adding
      a second production impl without a real consumer would be speculative.
- [ ] **Async I/O feature** (tokio) — DEFERRED to v0.6+. Preserving the
      "mutex never held across I/O" invariant under cancellation is a large
      design surface with no current consumer.
- [x] **XChaCha20-Poly1305 cipher** — DONE (see v0.5.0 batch above).

## Performance

Worked through in the 2026-07-20 PGO session. Outcomes below; numbers and
analysis in `docs/perf/2026-07-20_hot-path-flamegraph.md` and
`docs/perf/2026-07-20_read-from-scan-cache.md`.

- [x] **Profile-guided optimization of the hot path** — write-side
      `Compressor` pooling. `append/batch_1` 15.09 µs → 7.75 µs (2.07× faster).
- [x] **Consider `SmallVec<[T; 16]>` for `unflushed`** — REJECTED
      (regression on small batches).
- [x] **Bench `read_from` after the scan cache landed** — done.
- [x] **Pool the read-side zstd `DCtx`** — DONE 2026-07-20 (v0.5.0). The
      `Mutex<Decompressor>` is allocated once at `open()` and reused on every
      `read_from` / `for_each_from`. Falls back to `zstd::decode_all` for
      frames without a content-size header (legacy or externally-written
      files).
- [ ] **Streaming deserialise + early-stop at `limit`** — DEFERRED to
      v0.6. Blocked by ciborium's private `Deserializer` struct; the clean
      early-stop path requires either forking ciborium or changing the
      envelope. v2's item-count field retires this.

## Docs & polish

- [x] **Skill-contract debt — RENEGOTIATED.** The retrospective HTML
      artifacts required by `code-quality-scan`, `architecture-review`,
      `full-code-review`, and `nix-flake-migration` are NOT produced
      retroactively — point-in-time reports on past sessions rot fast and
      add no value. Each skill will produce its HTML artifact when next it
      applies to a fresh task. Future sessions: trigger the skill fresh, do
      not backlog its output.

## CI / tooling

- [x] **macOS flake verification** — DONE 2026-07-20. `.github/workflows/nix.yml`
      runs `nix flake check --no-build` on both `ubuntu-latest` and
      `macos-latest`. Catches aarch64-darwin regressions.
- [x] **Sign commits** — DONE 2026-07-20.
      `gpg.ssh.allowedSignersFile` configured globally at
      `~/.config/git/allowed_signers`. `git verify-commit HEAD` now succeeds
      ("Good 'git' signature for git@lars.software with ED25519 key…").
- [x] **Enable auto-merge for dependabot PRs** — DONE 2026-07-20.
      `gh repo edit --enable-auto-merge` set, and every Dependabot updater
      in `.github/dependabot.yml` now specifies `auto-merge.method: squash`.
      PRs auto-merge after required status checks pass.

## Crates.io publishing

- [x] **CARGO_REGISTRY_TOKEN wiring** — DONE 2026-07-20.
      `.github/workflows/publish.yml` is already fully wired (tag push
      triggers `cargo publish --features encryption` with the secret
      injected). `docs/RELEASE.md` § "Publish to crates.io" documents the
      one-time setup steps for the repo admin (create crates.io token, add
      as Actions secret). Until the secret is added by the user, the
      workflow's publish step fails with a clear error; manual `cargo
publish` is the fallback.

## Investigation

All entries resolved 2026-07-20. Findings recorded inline so the decisions
(especially the "no action needed" ones) are not re-litigated.

- [x] **Tighten `T: 'static`** — RELAXED (the bound was redundant).
- [x] **Extract AES-GCM cipher into its own feature/crate boundary** —
      NO ACTION: the feature boundary already achieves the goal.
- [x] **Profile the hermetic Nix build** — FIXED via
      `ZSTD_SYS_USE_PKG_CONFIG`.
- [x] **Investigate whether `include_str!("../README.md")` should be
      replaced** — REPLACED (removed the embedding).
- [x] **Consider `cargo supply-chain` crate** — ADDED as informational.
