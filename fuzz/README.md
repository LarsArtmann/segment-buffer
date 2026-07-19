# Fuzzing segment-buffer

Fuzz targets verify the crash-recovery and error-handling contracts: arbitrary
bytes on disk must never cause a panic.

## Running

Fuzzing requires a nightly Rust toolchain (for the sanitizers `libfuzzer-sys`
depends on).

### Option A — Nix (reproducible, recommended)

The repo's `flake.nix` ships both a `devShells.fuzz` and two `apps` that pin
the nightly toolchain and zstd:

```sh
# Interactive fuzz shell (manual cargo-fuzz invocation):
nix develop .#fuzz
# (then inside the shell) cargo-fuzz run fuzz_corrupted_read -- -max_total_time=60

# Or one-shot via apps (default 60s; pass a positional arg to override):
nix run .#fuzz-corrupted-read --        # 60s
nix run .#fuzz-corrupted-read -- 300    # 5 minutes
nix run .#fuzz-recovery
```

Both apps expect `cargo-fuzz` on `$HOME/.cargo/bin` (install once:
`cargo install cargo-fuzz`).

### Option B — rustup

```sh
rustup toolchain install nightly

# From the repo root:
cargo +nightly fuzz run fuzz_corrupted_read -- -max_total_time=60
cargo +nightly fuzz run fuzz_recovery       -- -max_total_time=60
```

## Verified locally

2026-07-19, via `nix develop .#fuzz`:

| Target                | Runs    | Time | Crashes | Coverage blocks |
| --------------------- | ------- | ---- | ------- | --------------- |
| `fuzz_corrupted_read` | 187,811 | 60s  | 0       | 392             |
| `fuzz_recovery`       | 942,719 | 60s  | 0       | —               |

libFuzzer recovered the `SBF1` magic dictionary entry organically from
`fuzz_corrupted_read`, confirming the envelope-detection path is exercised.

## Targets

| Target                | Contract                                                                                                       |
| --------------------- | -------------------------------------------------------------------------------------------------------------- |
| `fuzz_corrupted_read` | After overwriting an on-disk segment with arbitrary bytes, `read_from` returns `Err` and never panics.         |
| `fuzz_recovery`       | `SegmentBuffer::open` over a directory containing arbitrary files (valid, corrupt, or mis-named) never panics. |

Corpus and crash artifacts are written under `fuzz/<target>/` (gitignored).

## CI integration

Not yet wired. The plan is a nightly scheduled GitHub workflow
(`.github/workflows/fuzz.yml`) that runs both apps for ~5 minutes on every
midnight run, plus proptest analogues already run on every `cargo test`.
Tracked in [TODO_LIST.md](../TODO_LIST.md).
