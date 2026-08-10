## Performance tuning and documentation

The default zstd compression level changes from 3 to 1 based on a full compression-level sweep — level 1 is ~2x faster to encode with negligible ratio loss (3.2x vs 3.1x on text payloads). For a throughput buffer where segments are short-lived, encode speed matters more than ratio.

### ⚠️ Behavioral change: compression default 3 → 1

Existing segments at any compression level still decode correctly (the level is encode-only). New segments will use level 1 unless explicitly overridden. If you need the previous default:

```rust
let config = SegmentConfig::builder()
    .compression_level(3)
    .build();
```

### Highlights

- **Compression-level sweep**: tested all 22 zstd levels across 4 payload kinds. Level 1 is the sweet spot for a throughput buffer — 2x faster than level 3, negligible ratio loss. Levels above 10 collapse throughput to single-digit K items/s for negligible ratio gain.
- **Concurrent append benchmark**: `append_all` is 3.6x faster than `append` at 8 threads (7.1 vs 1.96 Melem/s) because it acquires the mutex once per batch.
- **LIMITATIONS.md**: comprehensive design-limitations documentation now visible on docs.rs via the crate-level rustdoc.
- **Scaling example**: latency percentiles (p50/p95/p99), real-disk testing (`--dir`), encrypted testing (`--encrypted`), and buffer retention (`--keep`).

### Full changelog

https://github.com/LarsArtmann/segment-buffer/blob/v0.5.7/CHANGELOG.md
