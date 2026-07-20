# Hot-path flamegraph: zstd CCtx pooling

**Captured:** 2026-07-20
**Method:** `RUSTFLAGS="-C force-frame-pointers=yes -g" cargo build --release --example hotpath_profile`, then `perf record -F 9999 --call-graph=fp` on the resulting binary. 27 240 samples pre-fix, 14 971 samples post-fix (same workload, fewer samples ⇒ less wall-clock time).
**Caveat:** single machine, single run, perf `--call-graph=fp` (frame-pointer-based stack walking). Numbers are indicative of direction and order-of-magnitude, not publication-grade.

## TL;DR

**66% of `flush` CPU was in `__memset_avx512_unaligned_erms`**, called from zstd's `ZSTD_CCtx_init_compressStream2`. The cause: `segment::encode_payload` called `zstd::encode_all` per flush, and `zstd::encode_all` constructs a fresh ~200 KB `CCtx` (and memsets it to zero) on **every** call. Pooling the `CCtx` through `zstd::bulk::Compressor` removed the per-flush memset entirely.

| Benchmark     | Before    | After     | Change                        |
| ------------- | --------- | --------- | ----------------------------- |
| `batch_1`     | 15.090 µs | 7.7495 µs | **−51.96%** (2.07× faster)    |
| `batch_100`   | 28.059 µs | 20.837 µs | **−24.06%** (1.32× faster)    |
| `batch_1000`  | 124.70 µs | 118.84 µs | −3.25% (within noise; ~1.03×) |
| `batch_10000` | 1.2062 ms | 1.0560 ms | **−10.28%** (1.11× faster)    |

CPU cycles for the same `examples/hotpath_profile.rs` workload dropped from **6.98 Gcycles → 1.68 Gcycles** (4.15× reduction).

## Pre-fix profile (top hot functions, self time)

```
66.13%  libc.so.6             __memset_avx512_unaligned_erms
 6.21%  hotpath_profile       FSE_buildCTable_wksp
 1.53%  libc.so.6             _int_malloc
 1.45%  libc.so.6             malloc_consolidate
 1.17%  hotpath_profile       HIST_countFast_wksp
 1.06%  hotpath_profile       ZSTD_compressBlock_doubleFast
 0.98%  hotpath_profile       segment_buffer::SegmentBuffer<T>::append
```

The call chain into the memset:

```
flush (96.74%)
└─ segment::write (92.46%)
   └─ zstd::encode_all (84.47%)
      └─ zstd::stream::write::Encoder::write (68.36%)
         └─ zstd_safe::CCtx::compress_stream (68.25%)
            └─ ZSTD_compressStream (67.88%)
               └─ ZSTD_CCtx_init_compressStream2 (67.70%   ← here)
                  └─ ZSTD_compressBegin_internal (67.00%)
                     └─ __memset_avx512 (66.13%)
```

`ZSTD_compressBegin_internal` initialises the `CCtx` for a new compression session. When the `CCtx` is freshly allocated (as `zstd::encode_all` does every call), this means memset'ing ~200 KB of internal tables to zero. When the `CCtx` is reused, zstd skips the memset and pays only the `SessionOnly` reset cost (~0.2% of CPU in the post-fix profile).

## Post-fix profile (top hot functions, self time)

```
24.70%  hotpath_profile       FSE_buildCTable_wksp
 4.36%  hotpath_profile       segment_buffer::SegmentBuffer<T>::append
 4.13%  hotpath_profile       ZSTD_compressBlock_doubleFast
 4.12%  hotpath_profile       segment_buffer::SegmentBuffer<T>::flush
 3.13%  libc.so.6             __memmove_avx512_unaligned_erms
 2.52%  libc.so.6             _int_malloc
 2.48%  libc.so.6             realloc
 2.44%  hotpath_profile       <alloc::string::String as core::fmt::Write>::write_char
 2.36%  hotpath_profile       HIST_countFast_wksp
 2.10%  hotpath_profile       alloc::fmt::format::format_inner
 1.69%  hotpath_profile       <std::path::Path>::_join
 1.39%  hotpath_profile       ZSTD_CCtx_init_compressStream2
 1.39%  hotpath_profile       ZSTD_resetCCtx_internal
```

The memset has dropped out of the top hits entirely. The remaining work is dominated by **actual compression** (FSE table build, doubleFast block compression, histogram counting) and unavoidable allocation overhead. `ZSTD_CCtx_init_compressStream2` and `ZSTD_resetCCtx_internal` are still present at ~1.4% each — that is the cheap session-reset path that replaces the expensive memset.

The `format_inner` / `write_char` / `pad_integral` cluster (~8% combined) is the `format!("payload-{i}")` string-building inside the benchmark harness itself — it is **not** segment-buffer code. A follow-up could replace the per-append `String` allocation with a reused buffer in `bench_append`, but that is benchmark hygiene, not a library win.

## The fix

`SegmentBuffer` now carries a `compressor: Mutex<zstd::bulk::Compressor<'static>>`, allocated once in `open_with_report` at the configured compression level. `flush` → `write_segment` locks the compressor, hands it to `segment::encode_payload`, which calls `compressor.compress(&cbor_buf)` instead of `zstd::encode_all(cbor_buf.as_slice(), level)`. The compressor's internal `CCtx` is reused across calls; zstd's `compress2` does the per-frame session reset internally.

Design notes:

- **Why a separate `Mutex`, not inside `BufferInner`?** `flush` releases `inner.lock()` before doing any I/O or compression (the "mutex never held across I/O" invariant). Putting the compressor inside `BufferInner` would either extend the mutex hold time through the compression step, or force an awkward `Option::take` + restore dance. A dedicated `Mutex<Compressor>` keeps the concern separate and the inner lock untouched.
- **Is the new mutex contended?** No. `flush` already takes `inner.lock()` briefly to drain pending events, and the re-entrancy guard serialises concurrent flushers against `for_each_from`. The compressor mutex is held only during the in-memory compression step, which is uncontended in practice.
- **Why not also pool the read-side `DCtx`?** Symmetry suggests `zstd::bulk::Decompressor` pooling would help `read_from`/`for_each_from` the same way. The flamegraph above only exercises the write path, so the read-side win is unmeasured. Deferred to a follow-up (see TODO_LIST.md).
- **`Send + Sync` preserved.** `zstd::bulk::Compressor` is `Send` (asserted in the zstd crate); `Mutex<Compressor>` is `Send + Sync`. The existing `const _: () = { assert_send_sync::<SegmentBuffer<()>>() }` compile-time check at the bottom of `lib.rs` still passes.

## Reproducing

```sh
# Build the hot-path driver with frame pointers.
RUSTFLAGS="-C force-frame-pointers=yes -g" cargo build --release --example hotpath_profile

# Profile.
sudo perf record -F 9999 --call-graph=fp -o /tmp/perf.data -- \
    ./target/release/examples/hotpath_profile

# Report (self time, top hits).
perf report -i /tmp/perf.data --stdio --no-children -g none --percent-limit 0.5

# Or, via cargo-flamegraph:
nix run nixpkgs#cargo-flamegraph -- --example hotpath_profile --release \
    -- -F 9999 --call-graph=fp
```

```sh
# A/B benchmark (pre-fix vs post-fix):
git stash push src/lib.rs src/segment.rs src/tests.rs src/property_tests.rs
cargo bench --bench bench_append -- --warm-up-time 1 --measurement-time 3   # baseline
git stash pop
cargo bench --bench bench_append -- --warm-up-time 1 --measurement-time 3   # criterion reports the delta
```

## What this is NOT

- Not a benchmark of the encryption feature — AES-GCM cost is paid only when `cipher` is `Some`, and all numbers above are default-features.
- Not a fix for the read path — `read_from` / `for_each_from` still pay a fresh `DCtx` per call. See `docs/perf/` for the cold-vs-warm scan-cache numbers that were captured alongside this fix.
- Not a reason to skip CI verification — these are local numbers from a single machine; the relative ordering is the durable claim, the absolute microseconds are not.
