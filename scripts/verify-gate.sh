#!/usr/bin/env bash
# scripts/verify-gate.sh
#
# The full local verification gate (AGENTS.md rules 4, 5, 6, 9).
# Runs every check CI runs, in the same spirit, and exits non-zero on the
# first failure OR after running all of them with a summary — see -a / --all.
#
# Usage:
#   scripts/verify-gate.sh            # stop on first failure (fast feedback)
#   scripts/verify-gate.sh --all      # run every gate, print a summary
#   scripts/verify-gate.sh --no-supply-chain   # skip cargo audit + cargo deny
#   scripts/verify-gate.sh --no-loom           # skip the loom gate
#   scripts/verify-gate.sh --no-lychee         # skip the markdown link check
#   scripts/verify-gate.sh --no-actionlint     # skip the GitHub workflow lint
#
# Tool availability: cargo fmt/clippy/test/doc come with the toolchain.
# cargo-deny, cargo-audit, lychee, and actionlint are invoked via
# `nix run nixpkgs#...` so the script works on a plain `nix develop` shell
# without global installs.

set -u

cd "$(dirname "$0")/.." || exit 1

STOP_ON_FIRST=1
RUN_SUPPLY_CHAIN=1
RUN_LOOM=1
RUN_LYCHEE=1
RUN_ACTIONLINT=1
for arg in "$@"; do
  case "$arg" in
    -a|--all) STOP_ON_FIRST=0 ;;
    --no-supply-chain) RUN_SUPPLY_CHAIN=0 ;;
    --no-loom) RUN_LOOM=0 ;;
    --no-lychee) RUN_LYCHEE=0 ;;
    --no-actionlint) RUN_ACTIONLINT=0 ;;
    -h|--help)
      sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

PASS=0
FAIL=0
FAILED_STEPS=()

run() {
  local name="$1"; shift
  printf '\n=== %s ===\n' "$name"
  if "$@"; then
    printf 'PASS: %s\n' "$name"
    PASS=$((PASS + 1))
    return 0
  fi
  local rc=$?
  printf 'FAIL (rc=%s): %s\n' "$rc" "$name" >&2
  FAIL=$((FAIL + 1))
  FAILED_STEPS+=("$name")
  if [[ "$STOP_ON_FIRST" == "1" ]]; then
    printf '\nverify-gate: stopping at first failure (use --all to run every gate).\n' >&2
    exit "$rc"
  fi
  return "$rc"
}

run "fmt"            cargo fmt --all -- --check
run "clippy(default)" cargo clippy --all-targets -- -D warnings
run "clippy(encryption)" cargo clippy --all-targets --features encryption -- -D warnings
run "clippy(fuzz)"   cargo clippy --all-targets --features fuzz -- -D warnings
run "test(default)"  cargo test --no-fail-fast
run "test(encryption)" cargo test --no-fail-fast --features encryption
run "doc"            env RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --features encryption
run "html_root_url"  scripts/check-html-root-url.sh

if [[ "$RUN_SUPPLY_CHAIN" == "1" ]]; then
  run "cargo-deny"  nix run nixpkgs#cargo-deny -- check
  run "cargo-audit" nix run nixpkgs#cargo-audit -- audit
fi

if [[ "$RUN_LOOM" == "1" ]]; then
  run "loom"        env RUSTFLAGS="--cfg loom" cargo test --features loom --test loom --release
fi

if [[ "$RUN_LYCHEE" == "1" ]]; then
  # Link-check every markdown file CI checks. Mirrors .github/workflows/ci.yml's
  # lychee job so anchor/link drift is caught locally, not just in CI.
  #
  # Transient failures: lychee hits live URLs (GitHub, docs.rs, crates.io) and
  # occasional 500/429/timeout responses DO happen even on green links. The
  # `.github/lychee.toml` config sets `max_retries = 1` so a single transient
  # blip is retried once. If this step still fails, re-run lychee standalone:
  #   nix run nixpkgs#lychee -- --config .github/lychee.toml '*.md' 'docs/**/*.md' 'fuzz/README.md'
  # A persistent failure on the SAME URL across 2+ standalone runs is a real
  # broken link; a one-shot failure that clears on re-run is transient.
  run "lychee" nix run nixpkgs#lychee -- --config .github/lychee.toml '*.md' 'docs/**/*.md' 'fuzz/README.md'
fi

# actionlint: YAML parse is the floor. Catches ${{ }} expression syntax errors,
# `needs:` cycle detection, deprecated/outdated action versions, and runner/os
# typos that the YAML parser accepts silently. Mirrors the CI `actionlint` job.
# Skip locally with --no-actionlint (e.g. offline run).
if [[ "$RUN_ACTIONLINT" == "1" ]]; then
  run "actionlint" nix run nixpkgs#actionlint -- .github/workflows/*.yml
fi

run "nix flake check"  nix flake check --no-build

printf '\n========================================\n'
printf 'verify-gate: %d passed, %d failed\n' "$PASS" "$FAIL"
if [[ "$FAIL" -gt 0 ]]; then
  printf 'Failed steps: %s\n' "${FAILED_STEPS[*]}"
  exit 1
fi
printf 'ALL GATES GREEN\n'
