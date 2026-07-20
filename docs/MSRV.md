# MSRV policy

segment-buffer's Minimum Supported Rust Version is **1.86**.

## What this means

- The crate compiles, tests pass, and clippy is clean on Rust 1.86.0.
- Bumping the MSRV is a **minor** semver bump (not major), but it is always
  documented in `CHANGELOG.md` under a dedicated `### MSRV` subsection.
- The MSRV is declared in `Cargo.toml` via `rust-version = "1.86"`.

## Why 1.86

- **Trait-upcasting coercion** stabilized in 1.86 — used by `CipherError::source()`
  to upcast `Arc<dyn Error + Send + Sync>` to `&dyn Error`. Before 1.86 the crate
  carried an `ErrorExt` workaround trait specifically to bridge this gap; that
  workaround was deleted when the MSRV moved to 1.86.
- **`criterion` 0.8** (the dev-only benchmark harness) requires rustc 1.86. The
  previous MSRV of 1.85 froze criterion at 0.5 and required an indefinite
  `dependabot.yml` ignore to suppress MSRV-violating bump proposals; bumping
  the MSRV to 1.86 retires that ignore and unblocks criterion 0.8+.
- **`rand` 0.10** ships on edition 2024 (rustc 1.85+); 1.86 covers it with margin.
- **Generic Associated Types (GATs)** stabilized in 1.65 — used implicitly by
  the lending-iterator shape of `for_each_from`.
- **`let-else`** stabilized in 1.65 — used in the segment parser.

## How to verify locally

### Via Nix (recommended — hermetic)

```bash
# Enter the MSRV shell (Rust 1.86.0 pinned via rust-overlay)
nix develop .#msrv

# Inside the shell, run the verification gate
cargo check --all-targets --features encryption
cargo test --no-fail-fast --features encryption
cargo clippy --all-targets --features encryption -- -D warnings
```

### Via rustup

```bash
rustup toolchain install 1.86.0
cargo +1.86.0 check --all-targets --features encryption
cargo +1.86.0 test --no-fail-fast --features encryption
cargo +1.86.0 clippy --all-targets --features encryption -- -D warnings
```

## CI verification

The `.github/workflows/ci.yml` workflow has a dedicated `msrv` job that runs
`cargo check --all-targets --features encryption` on Rust 1.86. Additionally,
the `test` job matrix includes `"1.86"` as one of the Rust versions, so the
full test suite runs on the MSRV on every push.

The `flake.nix` `packages.default` is also built with the MSRV-pinned crane
toolchain, so `nix build .#default` proves the package builds on 1.86.

## When to bump

Bump the MSRV when:

- A new dependency requires it (and there is no viable older alternative).
- A language feature would materially simplify the code.
- The Rust team's support policy makes the current MSRV untenable.

When you bump:

1. Update `Cargo.toml` `rust-version`.
2. Update `flake.nix` `devShells.msrv` and `packages.default` toolchain pin.
3. Update the CI matrix.
4. Delete any MSRV workaround that is no longer needed.
5. Add a `### MSRV` subsection to `CHANGELOG.md`.
6. Update this document.
