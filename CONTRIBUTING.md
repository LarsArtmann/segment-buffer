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

## Semver and stability policy

This crate follows [Semantic Versioning](https://semver.org/) with the
following concrete rules:

- **Breaking changes (major bump)**: adding a public item, removing a public
  item, changing a signature, changing on-disk format, changing the `Debug` /
  `Display` output of a public type in a way operators may be parsing, or
  changing behavior in a way that requires downstream action.
- **Additive changes (minor bump)**: adding a new public item, performance
  improvements, doc/test additions, new feature flags, bug fixes that do not
  change documented behavior.
- **Patch-level changes (patch bump)**: regression fixes, doc/test additions
  that don't change the API, dependency bumps within semver.

### `#[non_exhaustive]` is the default for new public structs and enums

Adding a field to a struct or a variant to an enum is otherwise breaking. We
avoid that by marking every new public struct/enum `#[non_exhaustive]` at
birth. `SegmentConfig`, `BufferStats`, and `SegmentError` are all
`#[non_exhaustive]` from v0.3.0 onward. The cost is paid by callers (they
must use `Default + field reassignment` or a builder), not by the API.

### `std::error::Error::source()` chains are required on wrapped errors

When a public error variant wraps a typed underlying error, the underlying
cause must be reachable via `Error::source()` — not erased behind a `format!`.
`CipherError::with_source` is the constructor that honors this; `msg` is
for errors with no underlying cause. Adding a typed underlying error and
_then_ removing it later (or vice versa) is breaking.

### On-disk format changes require a migration story

The on-disk format (filename contract, envelope, CBOR+zstd encoding, cipher
payload format) is part of the API. A change to it is a breaking change AND
requires either an automatic migration path or a documented upgrade
procedure that does not lose data. See the `SBF1` envelope (added in v0.2.0)
for the additive-change pattern: legacy files are auto-detected and read
without migration.

## Reporting Issues

Use [GitHub Issues](https://github.com/LarsArtmann/segment-buffer/issues).

For the current state of the project, see
[FEATURES.md](FEATURES.md) (capability inventory) and
[ROADMAP.md](ROADMAP.md) (direction).

## License

By contributing, you agree that your contributions will be licensed under the
Apache License 2.0.
