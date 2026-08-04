# segment-buffer Status Report — 2026-08-04 03:45 CEST

**Session topic:** Code duplication analysis and elimination  
**Working branch:** `master`  
**Commits ahead of origin:** `2` (`85f7f65` refactor dedup helpers, `ac629b8` CI fix)  
**Working tree:** clean (auto-git daemon committed the refactoring)  
**Latest CI on pushed commits:** `success` (CI + Nix + Publish all green on `ac629b8`)  
**WARNING:** Commit `85f7f65` has NOT been pushed yet — CI has not run on it.

---

## What triggered this session

The user asked: **"Do we have duplicate code?"** — with instructions to break
it down, execute systematically, and verify. After the user pointed out that
`art-dupl` (the deduplicate-code skill's tool) does not work on Rust, the
analysis was done manually by reading every source file.

---

## a) FULLY DONE

### Library-code deduplication (committed in `85f7f65`)

1. **`BufferInner<T>` helper methods extracted.** Three computations that were
   inlined at multiple call sites are now named methods on `BufferInner`:

   | Helper | Previously duplicated at | Now used by |
   |---|---|---|
   | `pending_count()` | `pending_count()`, `stats()` | both public methods |
   | `latest_sequence()` | `latest_sequence()`, `stats()` | both public methods |
   | `pending_start()` | `read_from`, `for_each_from` | both internal call sites |

2. **`compute_store_pressure()` extracted.** The `approx_disk_bytes /
   max_size_bytes` formula (with the `== 0` early return and `.min(1.0)`
   clamp) was duplicated verbatim in `store_pressure()` and `stats()`. Now
   both delegate to one private associated function.

3. **All 184 tests pass** (144 unit + 1 integration + 39 doc tests).
4. **Clippy clean** under both feature configurations (`--features encryption`
   and default).
5. **`cargo fmt --all -- --check`** passes.

### Verification gate run this session

| Gate | Command | Result |
|---|---|---|
| Compile | `cargo check --features encryption` | PASS |
| Clippy (encryption) | `cargo clippy --all-targets --features encryption -- -D warnings` | PASS |
| Clippy (default) | `cargo clippy --all-targets -- -D warnings` | PASS |
| Format | `cargo fmt --all -- --check` | PASS (after one `cargo fmt --all` run) |
| Tests | `cargo test --no-fail-fast --features encryption` | PASS (184/184) |

---

## b) PARTIALLY DONE

### `delete_acked` still inlines `pending_start`

The `delete_acked` method at `src/lib.rs` line ~1673 still has the old
hand-rolled expression:

```rust
let pending_start = inner
    .next_seq
    .saturating_sub(u64::try_from(inner.unflushed.len()).unwrap_or(u64::MAX));
```

This should be `inner.pending_start()` — the exact same refactor already
applied to `read_from` and `for_each_from`. It was missed because
`delete_acked` was the next item on the list when the session was interrupted.

**Severity:** Low (cosmetic — the formula is correct, just not DRY). The
refactor is a one-line replacement.

### No `publish_disk_stats` helper extracted yet

`sync_disk_bytes` and `recover` both do:

```rust
self.segment_count.store(
    u64::try_from(segments.len()).unwrap_or(u64::MAX),
    std::sync::atomic::Ordering::Relaxed,
);
```

with an accompanying `approx_disk_bytes.store(...)`. This was identified as
extractable but not yet implemented.

---

## c) NOT STARTED

### Test-code deduplication (the larger duplications)

The agent search revealed **massive** duplication in the test files. None of
this was touched:

1. **`property_tests.rs` has no test helpers.** It repeats:
   - `SegmentConfig { flush_policy: Manual, ..default() }` + `open()` — **11 times**
   - Concurrent test config (`Manual` + 100MB + `Throughput`) — **6 times**
   - `PropItem { id, payload: format!("payload-{id}") }` — **30+ times**
   - `.zst` file counting from directory — **5+ times**

   `tests.rs` already has `test_config()`, `test_buffer()`, `test_item()`, and
   `count_disk_segments()` helpers. `property_tests.rs` was written without
   them and never caught up.

2. **Concurrent reader/deleter/flusher thread patterns** are near-identical
   between `tests.rs` and `property_tests.rs` (5+ instances each), but each has
   test-specific assertions inline, making extraction non-trivial.

3. **`segment_size_stats` brute-force verification** is duplicated between
   `tests.rs` (2 instances: plaintext + encrypted) and `property_tests.rs`
   (1 instance).

### Acceptable duplications (identified, not touching)

These were analysed and deliberately accepted:

| Duplication | Why it stays |
|---|---|
| AES-GCM vs XChaCha20 cipher impls | Different AEAD types (`Aes256Gcm` vs `XChaCha20Poly1305`); the trait IS the abstraction. A shared helper would take more type params than the duplicated code has lines. |
| `read_from` vs `for_each_from` Phase 1 loop | Different iteration semantics: `read_from` pushes owned items into a `Vec`; `for_each_from` calls `f(seq, &item)` by reference. Abstracting loses `read_from`'s ability to own items without a clone. |
| Test `#![allow(...)]` lint blocks | Each module needs its own — this is how Rust lint scopes work. Cannot be shared. |
| Concurrent reader loops in tests | Each has test-specific inline assertions. A helper would need 8+ parameters. |

---

## d) TOTALLY FUCKED UP

### Nothing is fucked up.

No data loss, no broken builds, no reverts needed. The one issue (clippy
`missing_const_for_fn` on the two new `BufferInner` methods) was caught and
fixed immediately by adding `const fn`. Tests pass.

### Process miss: auto-git daemon committed before verification

The auto-git daemon committed `85f7f65` **before** I ran clippy. The first
clippy run found two `missing_const_for_fn` errors on the new methods. The
fix (adding `const`) was applied and the daemon re-committed, but the commit
message in `85f7f65` describes the refactor as complete when it was actually
clippy-broken at commit time. The working tree is now clean and clippy-green,
but the commit history does not capture the clippy fix as a separate commit
(the daemon folded it in).

**Lesson:** When the auto-git daemon is active, run the full verification gate
**before** making the last edit in a logical unit, so the daemon's commit
captures a clean state.

---

## e) WHAT WE SHOULD IMPROVE

1. **Run the gate after every logical change unit, not at the end.** The
   clippy errors would have been caught before the daemon committed.

2. **The deduplication analysis was thorough but the execution was
   incomplete.** The library refactoring is ~80% done (one missed call site
   + one unextracted helper), and the test deduplication (the largest
   duplication mass) was not started. A more disciplined approach would
   finish one category completely before moving to the status report.

3. **`property_tests.rs` needs the same helper discipline as `tests.rs`.**
   The 30+ inline `PropItem { id, payload: format!(...) }` constructions and
   11+ repeated config blocks are a readability and maintainability burden.
   Adding `prop_config()`, `prop_buffer()`, `prop_item()`, and
   `count_segments()` helpers would eliminate ~100 lines of noise.

4. **The commit `85f7f65` has not been pushed.** CI has not verified it.
   Before any release or further work building on it, push and confirm green.

---

## f) Up to 50 things to get done next

### Finish the current dedup work (high priority)

1. Replace `delete_acked`'s inline `pending_start` with `inner.pending_start()`
2. Extract `publish_disk_stats(segments)` helper for `sync_disk_bytes` + `recover`
3. Check if `stats()` still needs its `#[allow(clippy::as_conversions, ...)]` (the cast moved to `compute_store_pressure`)
4. Push `85f7f65` and verify CI is green
5. Run the loom gate: `RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release`

### Test-code deduplication (medium priority)

6. Add `prop_item(n)` helper to `property_tests.rs` (mirrors `test_item`)
7. Add `prop_config()` helper (Manual flush, configurable `max_size_bytes`)
8. Add `prop_buffer(dir)` helper (open with `prop_config`)
9. Add `concurrent_test_config()` helper (Manual + 100MB + Throughput + compression 1)
10. Add `count_segments(dir)` helper to `property_tests.rs`
11. Replace all 11 inline `SegmentConfig { Manual, ..default() }` + open blocks
12. Replace all 6 inline concurrent-test config blocks
13. Replace all 30+ inline `PropItem { ... }` constructions
14. Extract `segment_size_stats` brute-force oracle into a test helper
15. Consider sharing helpers between `tests.rs` and `property_tests.rs` via a `mod test_helpers`

### Further library dedup analysis (lower priority)

16. Check if `append` and `append_all` share the "check should_flush + flush" tail — consider extracting
17. Check if `write_segment` and `read_segment` share path-construction patterns
18. Check if the `segment::encode_payload` and `segment::decode_payload` cipher error mapping can share a helper
19. Review whether the `HookedStore` in tests.rs can reduce its boilerplate trait forwarding
20. Check examples/ for repeated patterns (config construction, drain loops)

### Verification and CI

21. Run `scripts/verify-gate.sh` (all 14 gates)
22. Run `nix flake check`
23. Push `85f7f65` + formatting fix and confirm CI green
24. Run `gh run list --limit 4` after push and confirm
25. Run the loom gate (item 5) — not part of the local gate but required before release claims

### Documentation

26. Update `AGENTS.md` test counts if the test helpers change the count
27. Consider documenting the `BufferInner` helper methods in the AGENTS.md architecture section
28. If test helpers are added, update the AGENTS.md "Code conventions" section

### Broader quality

29. Run `cargo doc --no-deps --features encryption` and check for warnings
30. Run the supply-chain gate: `cargo audit` + `cargo deny check`
31. Consider whether the concurrent reader/deleter/flusher patterns in tests warrant a test DSL
32. Check if `benches/` has duplicated setup code across the 8 benchmark targets
33. Check if `examples/` has duplicated config/drain-loop patterns
34. Review whether `fuzz/` targets share input generation that could be extracted
35. Consider a clippy custom lint or script that flags `next_seq.saturating_sub(...)` outside `pending_start()`

---

## g) Questions I cannot figure out myself

### 1. Should I continue with the test-code deduplication now, or was the library-code dedup sufficient for this session?

The library refactoring is ~80% complete (one missed call site + one
unextracted helper). The test deduplication is a much larger effort
(~100+ lines of noise across `property_tests.rs`). Do you want me to
finish both, or is the library refactoring the scope you had in mind?

### 2. The auto-git daemon already committed `85f7f65` — should I push it?

The commit has not been pushed and CI has not run on it. It is clippy-clean
and all tests pass locally, but per verification rule 9, a local-only green
is not a "done" claim. Should I push now, or wait until the remaining
dedup work is complete and push as a batch?

### 3. For the cipher implementations (AES-GCM vs XChaCha20), do you agree the duplication is acceptable?

Both impls follow the same `encrypt`/`decrypt` shape (random nonce → AEAD
encrypt → prepend nonce / split nonce → AEAD decrypt), but the types
(`Aes256Gcm` vs `XChaCha20Poly1305`, `Nonce` vs `XNonce`, different nonce
lengths) make a shared helper more complex than the duplicated code. I
accepted it as intentional. Do you agree, or do you want me to attempt an
extraction (e.g. via a sealed trait or a macro)?
