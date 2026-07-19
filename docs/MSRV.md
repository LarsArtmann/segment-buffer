# MSRV policy

segment-buffer's Minimum Supported Rust Version is **1.85**.

## What this means

- The crate compiles, tests pass, and clippy is clean on Rust 1.85.0.
- Bumping the MSRV is a **minor** semver bump (not major), but it is always
  documented in `CHANGELOG.md` under a dedicated `### MSRV` subsection.
- The MSRV is declared in `Cargo.toml` via `rust-version = "1.85"`.

## Why 1.85

- **Generic Associated Types (GATs)** stabilized in 1.65 — used implicitly by
  the lending-iterator shape of `for_each_from`.
- **`let-else`** stabilized in 1.65 — used in the segment parser.
- No features from 1.86+ are currently used. The pre-1.86 trait-upcasting
  workaround in `src/cipher.rs` (`ErrorExt`) exists specifically because the
  MSRV is 1.85 and trait-upcasting-coercion stabilized in 1.86.

When the MSRV moves to 1.86+, the `ErrorExt` trait in `cipher.rs` can be
deleted and `CipherError::source()` simplified to `self.source.as_deref()`.

## How to verify locally

### Via Nix (recommended — hermetic)

```bash
# Enter the MSRV shell (Rust 1.85.0 pinned via rust-overlay)
nix develop .#msrv

# Inside the shell, run the verification gate
cargo check --all-targets --features encryption
cargo test --no-fail-fast --features encryption
cargo clippy --all-targets --features encryption -- -D warnings
```

### Via rustup

```bash
rustup toolchain install 1.85.0
cargo +1.85.0 check --all-targets --features encryption
cargo +1.85.0 test --no-fail-fast --features encryption
cargo +1.85.0 clippy --all-targets --features encryption -- -D warnings
```

## CI verification

The `.github/workflows/ci.yml` workflow has a dedicated `msrv` job that runs
`cargo check --all-targets --features encryption` on Rust 1.85. Additionally,
the `test` job matrix includes `"1.85"` as one of the Rust versions, so the
full test suite runs on the MSRV on every push.

The `flake.nix` `packages.default` is also built with the MSRV-pinned crane
toolchain, so `nix build .#default` proves the package builds on 1.85.

## When to bump

Bump the MSRV when:

- A new dependency requires it (and there is no viable older alternative).
- A language feature would materially simplify the code (e.g. moving to 1.86
  to delete the `ErrorExt` workaround).
- The Rust team's support policy makes the current MSRV untenable.

When you bump:

1. Update `Cargo.toml` `rust-version`.
2. Update `flake.nix` `devShells.msrv` and `packages.default` toolchain pin.
3. Update the CI matrix.
4. Delete any MSRV workaround that is no longer needed (e.g. `ErrorExt`).
5. Add a `### MSRV` subsection to `CHANGELOG.md`.
6. Update this document.
