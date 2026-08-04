## segment-buffer v0.5.5 — 2026-08-04

Non-breaking release: panic-free public API, live `segment_count`, `segment_size_stats` tuning primitive, scan-cache TOCTOU fix, strict Clippy lint architecture, and expanded concurrency-property coverage. No API break, no on-disk format change, no new dependency.

### Highlights

- **Panic-free public API** — `for_each_from` no longer holds the buffer mutex across the user callback; the only panic path (`assert_not_reentered`) and the `IterationGuard` type are gone. Re-entrant calls from inside a callback (`append`, `stats`, `delete_acked`, …) are now safe.
- **Fixed `iter_from` sequence-number bug with gaps** — `iter_from` now delegates to `for_each_from`, so returned `(seq, item)` pairs are correct when deleted segments leave gaps.
- **Live `segment_count` in `BufferStats`** — tracked incrementally alongside `approx_disk_bytes`, recalibrated by `sync_disk_bytes`.
- **New `segment_size_stats()`** — on-demand size distribution (`count/min/max/mean/p50/p90`) of on-disk segment files; the tuning primitive for `FlushPolicy::Batch(N)`.
- **Scan-cache TOCTOU fix** — `mtime` is now captured _before_ the directory scan, so a concurrent rename cannot pair a stale post-rename mtime with a pre-rename segment list.
- **Strict Clippy lint architecture** — `Cargo.toml` denies `pedantic + nursery + restriction` lints; library code is fully clippy-clean under the strict set.
- **Expanded concurrency coverage** — added `for_each_from`/`iter_from` property tests under concurrent delete/flush, a high-concurrency `segment_count` stress test, and new loom tests for the scan-cache path.

### Full changelog

See [CHANGELOG.md](https://github.com/LarsArtmann/segment-buffer/blob/v0.5.5/CHANGELOG.md).
