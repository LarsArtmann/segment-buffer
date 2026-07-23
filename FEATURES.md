# Features

Honest inventory of what `segment-buffer` does, by status. Code is the source of
truth; this file tracks reality, not aspirations.

| Status               | Meaning                                                      |
| -------------------- | ------------------------------------------------------------ |
| FULLY_FUNCTIONAL     | Code present and working (tests pass, or exercised in prod). |
| PARTIALLY_FUNCTIONAL | Ships but has documented gaps or edge-case limitations.      |
| PLANNED              | Designed or discussed; no code yet.                          |
| WORTH_CONSIDERING    | Raw idea, not yet designed.                                  |

> **Versioning note.** Items marked _(v0.5.0)_ shipped in the v0.5.0 release
> tag. If you depend on `segment-buffer = "0.4"`, these items are not in the
> crate you resolved; upgrade to `"0.5"` to pick them up.

## Core queue

| Capability                                               | Status               | Notes                                                                                                                                                          |
| -------------------------------------------------------- | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Durable bounded queue (`SegmentBuffer<T>`)               | FULLY_FUNCTIONAL     | Generic over `T: Serialize + DeserializeOwned + Clone + Send`. Proven on 597M+ events in monitor365.                                                           |
| Append with sequence-number assignment (`append`)        | FULLY_FUNCTIONAL     | Sequence numbers computed atomically inside the mutex (concurrency bug fixed; see CHANGELOG).                                                                  |
| Batch append (`append_all`)                              | FULLY_FUNCTIONAL     | Single-lock batch primitive added in v0.4.1; assigns contiguous seqs under one lock acquisition.                                                               |
| Batch + interval auto-flush (`flush`)                    | FULLY_FUNCTIONAL     | Configurable via `FlushPolicy` (v0.4.0): `Batch`, `Interval`, `BatchOrInterval`, `Manual`. Replaces the silent-combine of two fields.                          |
| Config builder (`SegmentConfig::builder()`)              | FULLY_FUNCTIONAL     | Fluent builder added in v0.4.0; required because the struct is `#[non_exhaustive]`. `Clone` since v0.5.0 (cipher moved to `Arc`).                              |
| Owned-item iterator (`iter_from` → `SegmentIter`)        | FULLY_FUNCTIONAL     | _(v0.5.0)_ `iter_from(start, limit)` returns `(seq, item)` pairs; standard `Iterator` combinators work. `for_each_from` stays for the zero-copy lending path.  |
| Range read across disk + memory (`read_from`)            | FULLY_FUNCTIONAL     | Merges on-disk segments with the in-memory tail, in ascending sequence order.                                                                                  |
| Lending iterator (`for_each_from`)                       | FULLY_FUNCTIONAL     | Zero-clone in-memory reads; ~21× faster than `read_from` on 1k items (v0.4.0). Re-entrancy-guarded (panics on callback re-entry, v0.4.1).                      |
| Ack-based segment deletion (`delete_acked`)              | PARTIALLY_FUNCTIONAL | Removes flushed segment files only; unflushed in-memory items remain until flushed (documented, count stays honest via head_seq clamp).                        |
| Backlog size (`pending_count`, `len`, `is_empty`)        | FULLY_FUNCTIONAL     | `next_seq - head_seq`; honest even when acks race unflushed items. `len`/`is_empty` added as standard-collection aliases in 0.2.0.                             |
| Atomic snapshot (`stats` → `BufferStats`)                | FULLY_FUNCTIONAL     | Single-lock snapshot of pending/latest/head/next seq + disk bytes + pressure. Added in 0.2.0.                                                                  |
| Recovery report (`open_with_report`)                     | FULLY_FUNCTIONAL     | Returns `RecoveryReport` (segment_count, head_seq, next_seq, disk_bytes, removed_tmp_files). Added in v0.4.0.                                                  |
| Disk-byte resync (`sync_disk_bytes`)                     | FULLY_FUNCTIONAL     | Re-stats the directory to correct drift from external file manipulation. Added in v0.4.1.                                                                      |
| Accessors (`path`, `config`)                             | FULLY_FUNCTIONAL     | Expose the directory path and the opened config without `Debug`-parsing. Added in v0.4.1.                                                                      |
| Backpressure metrics (`store_pressure`, `is_overloaded`) | FULLY_FUNCTIONAL     | Ratio of `approx_disk_bytes` to `max_size_bytes`. Admission policy is caller-defined.                                                                          |
| `unflushed` Vec capacity recycling                       | FULLY_FUNCTIONAL     | _(unreleased)_ `flush()` reserves the previous batch's capacity on the fresh empty `unflushed`, avoiding ~log2(N) reallocs on the next batch. No API change.   |
| Single-process lock (`flock` at `open`)                  | FULLY_FUNCTIONAL     | _(v0.5.0)_ Exclusive `flock` on `<dir>/.segment-buffer.lock` via `fs4::FileExt::try_lock`. Fail-fast `SegmentError::Locked` on contention. Released on `Drop`. |
| Compile-time `Send + Sync` of `SegmentBuffer<T>`         | FULLY_FUNCTIONAL     | Static assertion in `lib.rs` — adding a non-thread-safe field fails the build. Added in 0.2.0.                                                                 |

## Storage format

| Capability                                   | Status               | Notes                                                                                                                                                                                                         |
| -------------------------------------------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| zstd + CBOR segment files                    | FULLY_FUNCTIONAL     | `seg_{start:012}_{end:012}.zst`, configurable `compression_level` (1-22).                                                                                                                                     |
| **Format envelope (`SBF1` magic + version)** | FULLY_FUNCTIONAL     | 8-byte header; forward-evolvable without breaking legacy readers.                                                                                                                                             |
| **Envelope hardening**                       | FULLY_FUNCTIONAL     | Reserved-bytes-zero check drops legacy _encrypted_ false-positive rate from 2⁻³² to 2⁻⁵⁶. Added in 0.2.0.                                                                                                     |
| **Legacy file compatibility (unencrypted)**  | FULLY_FUNCTIONAL     | Pre-envelope (monitor365) files auto-detected; zero migration. Covered by `legacy_envelopeless_file_still_reads`.                                                                                             |
| **Legacy file compatibility (encrypted)**    | FULLY_FUNCTIONAL     | Covered by `legacy_encrypted_file_without_envelope_still_reads` in 0.2.0 (previously untested).                                                                                                               |
| Filename-based crash recovery                | FULLY_FUNCTIONAL     | No WAL, no metadata DB. `open()` scans filenames to rebuild `head_seq`/`next_seq`.                                                                                                                            |
| Atomic write (tmp → fsync → rename)          | FULLY_FUNCTIONAL     | A crash never leaves a partial segment; `.tmp` debris is cleaned on `open()`.                                                                                                                                 |
| Configurable durability (`DurabilityPolicy`) | FULLY_FUNCTIONAL     | _(v0.5.0)_ `Maximal` (fsync file + dir after rename), `Segment` (fsync file only; default), `Throughput` (no fsync; cloud is durable).                                                                        |
| Mutex-never-held-across-I/O invariant        | FULLY_FUNCTIONAL     | `flush()` drops the lock before `write_segment`; `recover()` drops it across the `fs::metadata` loop (fixed in 0.2.0).                                                                                        |
| Scan cache (`scan_segments`)                 | FULLY_FUNCTIONAL     | Directory scan result cached; invalidated by every on-disk mutation. Added in v0.4.0.                                                                                                                         |
| Scan-cache mtime probe                       | FULLY_FUNCTIONAL     | _(v0.5.0)_ Capability-probed at `open()`; on capable fs, `scan_segments` invalidates the cache when the directory mtime moves.                                                                                |
| Atomic disk-byte tracking                    | FULLY_FUNCTIONAL     | `approx_disk_bytes` is `AtomicU64`; `flush()` no longer re-acquires the mutex to bump it. Added in v0.4.0.                                                                                                    |
| Crash-recovery limitation                    | PARTIALLY_FUNCTIONAL | Unflushed in-memory items are lost on crash (by design — durability requires flush).                                                                                                                          |
| Pluggable I/O (`SegmentStore` trait)         | PARTIALLY_FUNCTIONAL | _(v0.5.0)_ Trait + `RealStore` impl shipped; reachable externally only under the `loom` feature via `open_with_store`. A second production impl (S3, in-memory) is deferred until a concrete consumer exists. |

## Encryption

| Capability                                           | Status           | Notes                                                                                                                                                               |
| ---------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SegmentCipher` trait (pluggable AEAD)               | FULLY_FUNCTIONAL | Always available; bring any `Send + Sync + Debug` encrypt/decrypt impl.                                                                                             |
| `AesGcmCipher` (AES-256-GCM, random 12-byte nonce)   | FULLY_FUNCTIONAL | Behind the `encryption` feature. Byte-compatible with monitor365's segment format.                                                                                  |
| `XChaCha20Poly1305Cipher` (24-byte nonce)            | FULLY_FUNCTIONAL | _(v0.5.0)_ Behind the `encryption` feature. Installed for new buffers by `SegmentConfigBuilder::recommended_cipher(key)`. Constant-time in software, no AES-NI dep. |
| `SegmentCipher` stored as `Arc<dyn … + Send + Sync>` | FULLY_FUNCTIONAL | _(v0.5.0)_ Makes `SegmentConfig` + builder `Clone` (was `Box` pre-v0.5.0; breaking).                                                                                |
| Opaque `CipherError` with `source()` chaining        | FULLY_FUNCTIONAL | `with_source` preserves the underlying AEAD error. Added in 0.2.0 (breaking).                                                                                       |

## Concurrency & operations

| Capability                                            | Status           | Notes                                                                                                                                                                         |
| ----------------------------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MPMC via `parking_lot::Mutex`                         | FULLY_FUNCTIONAL | Multiple writers and readers; mutex never held across file I/O.                                                                                                               |
| `for_each_from` re-entrancy guard                     | FULLY_FUNCTIONAL | Panics with a clear message if a callback re-enters the buffer (v0.4.1).                                                                                                      |
| `tracing` instrumentation (`debug` / `info`)          | FULLY_FUNCTIONAL | Flush, delete, and recovery events are logged.                                                                                                                                |
| Typed errors with segment-path context                | FULLY_FUNCTIONAL | `Cbor`/`Cipher`/`Integrity` carry the offending file path + phase. `Io` carries `IoSite` (`Dir`/`Segment(PathBuf)`/`Unknown`) since v0.5.0 (was `Option<PathBuf>`; breaking). |
| `SegmentError::Locked { path }`                       | FULLY_FUNCTIONAL | _(v0.5.0)_ Distinct from `Io`; callers pattern-match lock contention without sniffing strings.                                                                                |
| Criterion benchmarks                                  | FULLY_FUNCTIONAL | 8 targets: append, read_from, read_vs_for_each, delete_acked, recover, stats, append_all, durability_policy.                                                                  |
| CI matrix (ubuntu/macos × stable/1.86, `-D warnings`) | FULLY_FUNCTIONAL | Dedicated MSRV (1.86) verification job.                                                                                                                                       |

## Testing & trust

| Capability                                 | Status           | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------ | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unit tests (84) + property tests (15)      | FULLY_FUNCTIONAL | CRUD, partial reads, limits, crash recovery, concurrency, error paths, envelope detection, encrypted-legacy read, append_all batch semantics, sync_disk_bytes, re-entrancy guard, IoSite, XChaCha20, flock contention. Includes two MPMC boundary stress tests proving `read_from` never corrupts under concurrent `delete_acked` (spurious Io) or concurrent `flush` (transient gaps). Counts verified by `grep -c '#\[test\]' src/tests.rs` (84) and `grep -c '#\[test\]' src/property_tests.rs` (15). |
| Doc tests (38)                             | FULLY_FUNCTIONAL | Every public method has a runnable example. Count is the `cargo test --features encryption` doctest-binary total.                                                                                                                                                                                                                                                                                                                                                                                        |
| Property tests (`proptest`, 15 properties) | FULLY_FUNCTIONAL | Filename bijection, payload bijection, envelope identity, encrypted roundtrip with varied key, corrupted-segment/recovery analogues, FlushPolicy::Manual never auto-flushes, read_from limit monotonicity, delete_acked pending_count monotone non-increasing, for_each_from ↔ read_from equivalence, sync_disk_bytes reconciliation, append_all boundary.                                                                                                                                               |
| Fuzz targets (`cargo-fuzz`)                | FULLY_FUNCTIONAL | Scaffold in `fuzz/`; verified locally via Nix `devShells.fuzz` (nightly + `libfuzzer-sys`). `fuzz_corrupted_read`: 187,811 runs / 60s, 392 coverage blocks, no crashes. `fuzz_recovery`: 942,719 runs / 60s, no crashes. Nightly scheduled CI workflow added in v0.4.1. Proptest analogues run on every `cargo test` as additional coverage.                                                                                                                                                             |
| Loom concurrency verification              | FULLY_FUNCTIONAL | 9 tests in `tests/loom.rs`. Covers the in-memory hot path (append / append_all / stats snapshot / writer+reader snapshot) AND, since v0.5.0, the `delete_acked` + `append` interleaving (4 tests exhaustively enumerating every two-thread schedule via a loom-aware `MockStore`). `flush`/`recover`/`read_from` still covered statistically by the stress test `concurrency_4_writers_1_reader_10k_events` in `src/tests.rs`.                                                                           |
| Allocation-count regression guard          | FULLY_FUNCTIONAL | `tests/alloc_guard.rs`: a counting allocator asserting fixed heap-allocation budgets on the hot paths (warm append: 1, read_from in-memory: 3, stats: 1, append+flush: 32). Machine-independent — catches tail-latency regressions (extra clones, Vec growth) without CI hardware variance.                                                                                                                                                                                                              |

## Supply chain & ops

| Capability                             | Status           | Notes                                                                                           |
| -------------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------- |
| cargo-deny config                      | FULLY_FUNCTIONAL | `deny.toml`: advisories + licenses + bans + sources. Added v0.4.0.                              |
| cargo-audit in CI                      | FULLY_FUNCTIONAL | Dedicated `supply-chain` job in CI. Added v0.4.1.                                               |
| cargo-deny in CI                       | FULLY_FUNCTIONAL | Same `supply-chain` job. Added v0.4.1.                                                          |
| `cargo supply-chain publishers` report | FULLY_FUNCTIONAL | _(v0.5.0)_ Weekly informational workflow surfacing crates.io publisher attribution. Non-gating. |
| Dependabot + Renovate                  | FULLY_FUNCTIONAL | Belt-and-braces dep update configs. Added v0.4.1. Dependabot auto-merge enabled v0.5.0.         |
| Nix flake CI                           | FULLY_FUNCTIONAL | `.github/workflows/nix.yml`: flake check + build + test + fmt on ubuntu + macOS.                |
| Signed commits                         | FULLY_FUNCTIONAL | _(v0.5.0)_ `git verify-commit HEAD` succeeds via `gpg.ssh.allowedSignersFile`.                  |
| Nightly fuzz CI                        | FULLY_FUNCTIONAL | `.github/workflows/fuzz.yml`: scheduled cargo-fuzz runs. v0.4.1.                                |
| Flake.lock auto-update                 | FULLY_FUNCTIONAL | `.github/workflows/update-flake-lock.yml`: weekly PR. v0.4.1.                                   |

## Documentation & examples

| Capability                                                | Status           | Notes                                                                                                                                                                   |
| --------------------------------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Performance tuning guide (`docs/PERFORMANCE.md` § Tuning) | FULLY_FUNCTIONAL | _(unreleased)_ Impact-ordered guide to the four config-only Tier 0 levers: `DurabilityPolicy`, `FlushPolicy` + `append_all`, `compression_level`, `for_each_from`.      |
| `examples/background_flush.rs`                            | FULLY_FUNCTIONAL | _(unreleased)_ Recommended pattern for p99-sensitive producers: `FlushPolicy::Manual` + caller-owned timer thread. Demonstrates shutdown-flag + final-sync-before-exit. |

## Planned / worth considering

See [ROADMAP.md](ROADMAP.md) for long-term direction (async I/O, envelope v2 with streaming deserialise + Blake3 checksum + compression negotiation, a second `SegmentStore` impl, streaming cipher) and [TODO_LIST.md](TODO_LIST.md) for short/mid-term work.
