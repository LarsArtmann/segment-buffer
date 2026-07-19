# Fuzzing segment-buffer

Fuzz targets verify the crash-recovery and error-handling contracts: arbitrary
bytes on disk must never cause a panic.

## Running

Fuzzing requires a nightly Rust toolchain (for the sanitizers `libfuzzer-sys`
depends on):

```sh
rustup toolchain install nightly

# From the repo root:
cargo +nightly fuzz run fuzz_corrupted_read -- -max_total_time=60
cargo +nightly fuzz run fuzz_recovery       -- -max_total_time=60
```

## Targets

| Target | Contract |
| --- | --- |
| `fuzz_corrupted_read` | After overwriting an on-disk segment with arbitrary bytes, `read_from` returns `Err` and never panics. |
| `fuzz_recovery` | `SegmentBuffer::open` over a directory containing arbitrary files (valid, corrupt, or mis-named) never panics. |

Corpus and crash artifacts are written under `fuzz/<target>/` (gitignored).
