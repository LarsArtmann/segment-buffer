# What `segment-buffer` can learn from `croc`

**Created:** 2026-07-20
**Author:** Crush (glm-5.2), prompted by Lars
**Subject:** [`schollz/croc`](https://github.com/schollz/croc) v10.x (Go, MIT) — a relay-mediated, PAKE-secured, resumable peer-to-peer file transfer CLI.
**Status:** Research note. Not a release artifact. No code changed.

---

## TL;DR

**Most of `croc` is irrelevant to us — and that is the most useful thing about studying it.** `croc` is a networked, two-party, E2E-encrypted file-transfer CLI; `segment-buffer` is a single-process, local, at-least-once disk buffer. Their overlap is narrow: both ship atomic writes, pluggable AEADs, filename/state-in-the-data recovery, and typed errors. On those four axes `croc` is **independent confirmation that we chose well** — it arrived at the same shapes from a different starting point.

The genuinely _new_ lessons are few and they all live in **one place we currently under-serve: the cloud-sync drain-loop example code.** `croc`'s reconnect machinery (exponential backoff with state reset, transient-vs-fatal error classification, first-error-wins across parallel workers) is a mature, battle-tested template for exactly the loop our `examples/cloud_sync.rs` (TODO M4) must teach. One forward-looking note applies to the planned streaming/incremental cipher (TODO: streaming deserialise / RFC 8450).

Everything else — PAKE, relay, peer discovery, multi-stream multiplexing, CLI/UX, token-bucket throttling, path-traversal defence — belongs to a different problem and is explicitly rejected below so it is not re-litigated.

---

## Read this first: the domain gap

`croc` solves: _"get bytes from computer A to computer B when neither runs a server, both sit behind NAT, and the users are non-technical."_ Its hard problems are **networking** (relay matchmaking, NAT traversal, IPv6-first racing, parallel-stream multiplexing) and **human factors** (zero-config codephrases, clipboard, QR, tab completion).

`segment-buffer` solves: _"spool items to a local directory as fast as possible, drain them to a cloud endpoint later, never lose an unacked item, recover from crash by listing the directory."_ Its hard problems are **local concurrency** (a mutex invariant proven under loom) and **crash semantics** (filename-as-WAL, configurable durability).

The Venn intersection is: atomic persistence, pluggable AEADs, recovery-without-metadata, and typed retry-classifiable errors. **The rest of this document stays inside that intersection.** Anything outside it is in §5.

A useful sanity check: `croc` is ~35k stars and ~2,236 commits of _networking_ code. If this report found more than a handful of transferable lessons, that would be a signal we were scope-creeping, not learning.

---

## 1. Affirmations — things `croc` got right that we **also** got right

These are listed because independent convergence is stronger evidence than either project alone. None require action; all are worth a sentence in the relevant design doc the next time it is touched.

### 1.1 Atomic write via tmp → fsync → rename

- **croc:** writes chunks directly to the final file with `WriteAt` + pre-allocated `Truncate`, and uses gap detection for resume (see §3.1). For _its_ ephemeral temp files (stdin captures, zip archives) it uses a `croc-marked-files.txt` sidecar registry cleaned on next startup.
- **segment-buffer:** `RealStore::write_atomic` does `tmp → write_all → sync_all → rename` (`src/store.rs`), and treats `*.tmp` as crash debris swept on `open()` (`clean_tmp`). No sidecar registry — the suffix _is_ the registry.
- **Verdict:** our shape is the cleaner of the two. `croc`'s marked-file sidecar is a weaker invariant (a separate file that can itself be lost in the same crash that orphaned the temps). The suffix convention is self-describing and needs no companion state. **No action.** If anything, this is a small argument _against_ ever introducing a sidecar metadata file for any reason.

### 1.2 Recovery state lives in the data, not in a metadata DB

- **croc:** the "room" (rendezvous key) is `SHA256(codephrase[:4] + "croc")` — two parties with the same code find each other with zero coordination. Chunk positions are carried in-band as 8-byte LE offsets. No out-of-band manifest.
- **segment-buffer:** the filename `seg_{start:012}_{end:012}.zst` **is** the WAL. `head_seq` and `next_seq` are rebuilt by listing the directory.
- **Verdict:** same principle, independently discovered. Our version is stronger: `croc`'s state is recoverable from in-band bytes plus a known hash salt; ours is recoverable from filenames alone with no salt, no key, no parsing ambiguity. **No action.** This is the load-bearing design choice of the whole crate; `croc`'s convergence is evidence it generalises.

### 1.3 Pluggable AEAD behind a trait, with a legacy byte-compat constraint

- **croc:** `crypt.go` ships AES-256-GCM (default, PBKDF2 key derivation) and ChaCha20-Poly1305 (Argon2id). Both write `[nonce][ciphertext + tag]`. Selection is by codepath, not a trait.
- **segment-buffer:** `SegmentCipher` trait (always exported); `AesGcmCipher` behind `encryption`. Planned `XChaCha20Poly1305Cipher` (TODO M6). Same `[nonce][ciphertext+tag]` on-disk shape; AES-GCM stays byte-compatible with monitor365.
- **Verdict:** we are already ahead on the abstraction (`croc`'s ciphers are not polymorphic; selection is branched in call sites). `croc`'s ChaCha20 picks the **12-byte nonce** variant — the same 2³²-message-limit-per-key problem our TODO already flags as the reason to prefer **XChaCha20** (24-byte nonce). **No action; confirms M6's XChaCha20-over-ChaCha20 choice.**

### 1.4 Magic bytes for format self-identification

- **croc:** every wire frame is `[4-byte "croc" magic][4-byte LE length][payload]`. Non-matching magic ⇒ protocol desync, detected immediately. `maxReadMessageSize = 64 MiB` guards against memory-exhaustion via a forged length.
- **segment-buffer:** `SBF1` 8-byte envelope (`src/segment.rs`). Magic + version + 3 reserved bytes. Reserved-bytes-zero requirement drops the legacy-encrypted false-positive rate to 2⁻⁵⁶.
- **Verdict:** we use magic for _format detection_ (enveloped vs legacy), not for _stream desync detection_ — because we don't stream. `croc`'s length-prefix + max-size guard is a pattern worth remembering **only if** we ever adopt a streaming/incremental cipher (see §3.2). **No action today.**

### 1.5 Opaque errors with a chainable source

- **croc:** `CipherError` analogue is implicit; `crypt.go` returns `fmt.Errorf` strings. (Actually weaker than ours.)
- **segment-buffer:** `CipherError` carries `Arc<dyn Error + Send + Sync>` so the underlying AEAD error is preserved through `source()`. `SegmentError` is a typed enum with path + phase context.
- **Verdict:** ours is stronger. **No action.**

---

## 2. Genuinely new lessons — all in the drain-loop example code

`croc`'s transfer machinery is the single most engineering-mature part of the project, and it maps almost exactly onto the cloud-sync drain loop our README promises but no runnable example currently teaches (TODO **M4 — `examples/cloud_sync.rs`**, plus **M9** disk-full and **M10** idempotent-server examples). The layer split (AGENTS.md §"Layer split vs monitor365") means **these patterns belong in example/teaching code, not in the library** — but the example is currently the weakest part of our cloud-sync story, and `croc` is a high-quality template for it.

### 2.1 Exponential backoff with **state reset that preserves durable state**

This is the single best idea to copy. `croc`'s `transferWithReconnect()` (`croc.go`) is an outer retry loop:

1. `delay := reconnectBackoff(attempt)` — 100 ms → 200 ms → 400 ms → … capped at 5 s.
2. `resetForReconnectAttempt(attempt)` — clears all transient state (step booleans, PAKE instance, chunk counters) **but preserves** `FilesToTransfer`, `FilesHasFinished`, `FilesToTransferCurrentNum` so the transfer resumes rather than restarts.
3. Re-establish the connection; retry.

The principle, restated for our domain: **on a transient cloud failure, reset the in-flight HTTP request, the batch cursor, and the backoff window — but never reset `next` (the sequence cursor) or call `delete_acked` for a batch the server never confirmed.** Our at-least-once model already enforces this _structurally_ (the cursor is the consumer's, and `delete_acked` is the commit point), but the example should _demonstrate_ it visibly: a flaky uploader that fails the first two attempts per batch, then succeeds, with the cursor advancing only after success.

> **Action — TODO M4.** The planned `examples/cloud_sync.rs` `FlakyUploader` (M4.4) should implement `croc`-style exponential backoff with a clear comment: _"transient error → backoff and retry the SAME batch; never advance the cursor past an unacked batch."_ This is the at-least-once invariant expressed as runnable code.

### 2.2 Transient-vs-fatal error classification (`isFatalSenderRouteError`)

`croc` separates errors into two classes via `errors.As()` against sentinel types (`transferDisconnectError`, `pakeHandshakeError`):

- **Transient** (disconnect, timeout, relay reset) ⇒ retry with backoff.
- **Fatal** (bad password, auth refused, file refused) ⇒ bail immediately, do not retry.

Our drain loop needs the same split, and crucially the library _cannot make it for the consumer_: whether a cloud 4xx is fatal (bad auth → stop) or transient (429 → back off) is a property of the consumer's cloud contract, not of `SegmentError`. The example should show the pattern and name it.

> **Action — TODO M4 + a doc paragraph.** `examples/cloud_sync.rs` should define a `CloudError` enum with `Transient` / `Fatal` variants wrapping the underlying HTTP/transport error, and demonstrate `match` driving the retry decision. Cross-link this from README §"Cloud sync" so the pattern is discoverable. **Do not** add retry classification to `SegmentError` itself — that would pull cloud semantics into the crate (violates the layer split).

### 2.3 First-error-wins across parallel workers (`sync.Once` + buffered channel)

`croc` runs N parallel data streams (one goroutine per relay port) and aggregates their errors via a per-attempt struct: a buffered channel of capacity 1, a `sync.Once` that ensures only the _first_ error propagates, and a control-connection close that cascades cancellation to the other workers.

Our library is single-mutex and does not run parallel internal workers, so this is **not an internal pattern for us**. But it is the right pattern for a _multi-worker drain_ — e.g. sharding the buffer across N uploaders by sequence range — which a sophisticated consumer (monitor365 at scale) might well build. The example doesn't need to demonstrate it (single-worker is the right default to teach), but it should be **mentioned** in the example's doc comment as the canonical upgrade path, with a one-line Rust sketch (`Arc<AtomicBool>` "first failure wins" flag, or `tokio::select!` on the worker join handles).

> **Action — TODO M4 (doc comment only).** Add a `// Scale-up note:` comment in `examples/cloud_sync.rs` pointing at the first-error-wins pattern for multi-worker drains. No library change.

### 2.4 Clean shutdown signal handling (`ctx.go`)

`croc`'s `main.go` (55 lines) captures `SIGINT`/`SIGTERM`, runs the transfer in a goroutine, and on signal calls `utils.RemoveMarkedFiles()` then cancels a context that cascades through every loop via `ctxErr()` checks.

Our library is synchronous and owns no long-running loop, so there is no internal shutdown to coordinate. **But the drain-loop example should show clean shutdown:** install a SIGINT handler, stop starting new batches, let the in-flight batch finish (so `delete_acked` runs and the cursor advances), then exit. The at-least-once invariant is preserved automatically because unacked batches stay on disk; the example's job is to _show_ that, not to build machinery for it.

> **Action — TODO M4.** Add a `#[cfg(unix)]` SIGINT handler to `examples/cloud_sync.rs` (or `examples/cloud_sync_disk_full.rs`, M9) demonstrating graceful drain-shutdown: stop accepting new batches, finish the in-flight one, exit. Keep it to <30 lines.

---

## 3. Forward-looking notes — for work not yet scheduled

### 3.1 Direct-write + gap detection vs tmp-rename: a real fork, correctly resolved

`croc` writes chunks **directly to the final file** via `WriteAt`, pre-allocating the full size with `Truncate(fileSize)`. On resume it scans for all-zero gaps (`utils.MissingChunks`) to find what was never written. This is the _opposite_ of our `tmp → fsync → rename`.

Both are correct **for their workload**:

| Workload                                                                                   | Right choice                          | Why                                                                                                                     |
| ------------------------------------------------------------------------------------------ | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `croc`: one large file, built incrementally over an unreliable link, resumable across runs | Direct `WriteAt` + zero-gap detection | The file is mutable and partial; tmp-rename would need a file-sized staging copy and a directory-fsync per chunk        |
| `segment-buffer`: many small immutable segments, written once, never modified              | `tmp → fsync → rename`                | Segments are write-once; rename gives cheap atomicity; a crash never produces a _partial_ segment, only a `.tmp` orphan |

**The lesson is meta, not actionable:** the choice of crash-safety primitive is a function of the write pattern, not a universal ranking. We should not "upgrade" to `WriteAt`+gaps; segments are immutable so there is nothing to resume. Documented here so the question does not come back.

### 3.2 Chunk-range compression — relevant only if we adopt a streaming cipher

`croc`'s `MissingChunks` returns a compressed format `[chunkSize, start, count, start, count, …]` instead of a list of individual positions. This keeps the "what's missing" control message small even for files with millions of chunks.

We have no analogue because our segments are atomic: `read_from` returns a whole segment or nothing, and resume granularity is the segment. **This changes only if we adopt a streaming/incremental cipher** (TODO: "Streaming/incremental cipher — long-term. Likely v0.6+"). A chunked AEAD format (e.g. RFC 8450) would make a single segment partially-recoverable, at which point a `[start, count, …]` range encoding for "which chunks survived" becomes the right shape.

> **No action now.** When the streaming-cipher TODO is promoted to a real design (v0.6+), revisit this section. Until then, segment atomicity makes it irrelevant.

### 3.3 Multiple hash algorithms with speed/accuracy tradeoffs

`croc` defaults to `xxhash` (fast, non-cryptographic), offers `imohash` (samples 128 KB — extremely fast for huge files), and `highwayhash`/`md5` as alternatives. The principle: pick the hash that matches the access pattern, not the strongest hash.

Our planned per-segment **Blake3** checksum (TODO M17) is fast _and_ cryptographic, so we sidestep the tradeoff `croc` has to navigate. **No action; M17's choice is already the right one.** Worth noting in M17's eventual design note that Blake3 was chosen over CRC32/xxhash precisely because it gives us bit-rot detection _and_ integrity at stream speed, removing the need for a hash menu.

---

## 4. What we should NOT copy (anti-scope-creep register)

Listed so the question stays answered. Each item names the principle it would violate.

| `croc` feature                                                             | Why we reject it                                                                                                                                              | Principle violated                                      |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **PAKE key exchange** (`schollz/pake`)                                     | We are not a two-party protocol; the key is configured at `open()`.                                                                                           | Layer split — no protocol negotiation in a local buffer |
| **Relay matchmaking + room model**                                         | No second party to match.                                                                                                                                     | Single-process invariant                                |
| **Peer discovery (UDP multicast)**                                         | No network.                                                                                                                                                   | Single-process invariant                                |
| **Multi-stream multiplexing** (one goroutine per relay port)               | One disk, one mutex. Parallelism is the caller's concern (`append_all`, multi-thread producers).                                                              | Mutex-never-held-across-I/O invariant                   |
| **IPv6-first connection racing**                                           | No connections.                                                                                                                                               | —                                                       |
| **Token-bucket upload throttling** (`golang.org/x/time/rate`)              | We ship metrics-not-policy; throttling is the upstream consumer's concern (AGENTS.md §"Backpressure").                                                        | Metrics-not-policy                                      |
| **CLI/UX** (codephrases, clipboard, QR, tab completion)                    | We are a library.                                                                                                                                             | —                                                       |
| **Path-traversal defence** (`normalizeReceiveFilePath`, symlink rejection) | We are not a receiver; we write only to our own directory under our own naming scheme.                                                                        | —                                                       |
| **`recover()` from panics in goroutines**                                  | Rust threads panic independently; `catch_unwind` at the FFI boundary is the equivalent and is the caller's concern, not the library's.                        | —                                                       |
| **DEFLATE-at-HuffmanOnly compression**                                     | We use zstd, which already dominates DEFLATE on ratio at comparable speed. The "fastest possible, ratio-be-damned" knob is `compression_level` in our config. | Already covered                                         |
| **DNS resolver racing** (18 public resolvers in parallel)                  | No DNS.                                                                                                                                                       | —                                                       |
| **ChaCha20 (12-byte nonce)** as the extended-nonce option                  | We want XChaCha20 (24-byte nonce) to escape the 2³²-message limit — already in TODO M6.                                                                       | Already decided                                         |
| **`Arc<dyn Error>`-style string errors from `crypt.go`**                   | Our `CipherError` is already opaque-with-source, which is stronger.                                                                                           | Already ahead                                           |

---

## 5. Proposed deltas to `TODO_LIST.md`

These are proposals for the user to accept or reject. They are deliberately small and all live in already-planned items — **no new work is invented** by studying `croc`.

| Proposal                                                                          | Affects                                                   | Change                                                                                                                                                                                                                                                                              |
| --------------------------------------------------------------------------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M4 sub-task: backoff + transient/fatal classification in `examples/cloud_sync.rs` | TODO **M4** (`examples/cloud_sync.rs`)                    | Add explicit sub-tasks: (a) `CloudError { Transient, Fatal }` enum, (b) exponential backoff (100 ms → 5 s cap), (c) `FlakyUploader` that fails twice then succeeds, (d) `#[cfg(unix)]` SIGINT graceful-shutdown, (e) `// Scale-up note:` comment for multi-worker first-error-wins. |
| Cross-link from README §"Cloud sync"                                              | README                                                    | One sentence: _"For retry/backoff and transient-vs-fatal error classification in the drain loop, see `examples/cloud_sync.rs`."_                                                                                                                                                    |
| Streaming-cipher design note seeds                                                | TODO "Streaming/incremental cipher"                       | Add a one-line pointer: _"See `docs/CROC_LESSONS.md` §3.2 on chunk-range compression for partial-segment resume."_                                                                                                                                                                  |
| Affirmation references                                                            | AGENTS.md §"Crash recovery", §"Encryption on-disk format" | Optional: a single sentence each noting that `croc` independently converges on tmp-rename atomicity and `[nonce][ciphertext+tag]` AEAD shape. Low value; skip unless AGENTS.md is edited for another reason.                                                                        |

**Nothing else changes.** In particular: no new dependencies, no new public API, no change to `SegmentError`, no change to the storage format, no change to the layer split.

---

## 6. Methodology and limits of this report

- **Sources:** `croc` v10.x `src/{croc,comm,tcp,crypt,message,compress,models,utils,cli}/` read via the GitHub mirror; author's design blog (`infinitedigits.co/croc`); DeepWiki architecture/security pages. No `croc` code was run.
- **`segment-buffer` sources:** `AGENTS.md`, `TODO_LIST.md`, `ROADMAP.md`, `FEATURES.md`, `README.md`, `docs/planning/2026-07-20_03-40_v0.5.0-cloud-sync-throughput-batch.md`, and `src/{lib,store,segment}.rs` (the flush path, `write_atomic`, and the encode/decode pipeline).
- **No measurements were taken.** Every `croc` claim is paraphrased from its source; every `segment-buffer` claim is grounded in the files above. No "was X, now Y" baseline is asserted.
- **This is a point-in-time research note**, not a living document. It will go stale as both projects evolve; treat the §2 action proposals as the durable output and everything else as rationale.
- **The strongest claim in this report is structural, not technical:** two unrelated projects converging on tmp-rename atomicity, state-in-filenames, pluggable AEADs, and typed retry-classifiable errors is evidence those four choices generalise beyond either domain. That is worth more than any individual code pattern.
