# Contributing

Thanks for your interest in contributing to segment-buffer!

## How to Contribute

1. Fork the repository
2. Create a feature branch (`git switch -c my-feature`)
3. Make your changes
4. Ensure the full verification suite below passes
5. Submit a pull request

## Development Setup

The `encryption` feature is **off by default**, so most commands must run twice —
once with default features and once with `--features encryption`. CI does exactly
this (see `.github/workflows/ci.yml`).

### Using Cargo directly

```bash
# Tests (canonical command — runs both default + encryption-gated tests)
cargo test --no-fail-fast --features encryption

# Lint (warnings are hard errors, both in CONTRIBUTING and CI via RUSTFLAGS=-D warnings)
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features encryption -- -D warnings
cargo fmt --all -- --check

# Examples
cargo run --example basic_usage
cargo run --example backpressure
cargo run --example encrypted --features encryption   # REQUIRES the feature flag

# Benchmarks (criterion)
cargo bench --bench bench_append
cargo bench --bench bench_read_from
cargo bench --bench bench_delete_acked
cargo bench --bench bench_recover

# Docs (build with the feature so AesGcmCipher is visible)
cargo doc --no-deps --features encryption
```

### Using Nix (reproducible)

A `flake.nix` provides a devShell with the pinned Rust toolchain plus the native
`zstd` and `pkg-config` dependencies:

```bash
nix develop          # drop into the devShell
nix flake check      # verify the flake
```

## Code Conventions

- `#![warn(missing_docs)]` is on — every public item needs a doc comment.
- Doc comments use `# Errors` and `# Example` sections where relevant.
- Tests use `tempfile::TempDir`; see `src/tests.rs` for the helper patterns.
- The MSRV is **1.85** (enforced by a dedicated CI job).

## Reporting Issues

Use [GitHub Issues](https://github.com/LarsArtmann/segment-buffer/issues).

For the current state of the project, see
[FEATURES.md](FEATURES.md) (capability inventory) and
[ROADMAP.md](ROADMAP.md) (direction).

## License

By contributing, you agree that your contributions will be licensed under the
Apache License 2.0.
