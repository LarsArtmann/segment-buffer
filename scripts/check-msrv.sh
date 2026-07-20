#!/usr/bin/env bash
# scripts/check-msrv.sh
#
# Asserts that the declared MSRV is consistent across every place it appears:
#   1. Cargo.toml          (rust-version = "X.Y" — the canonical source)
#   2. .github/workflows/ci.yml  (matrix entry + msrv job toolchain)
#   3. flake.nix            (msrv devShell pin + header comment)
#   4. docs/MSRV.md         (headline)
#
# MSRV drift (one location bumped, others forgotten) caused 5+ consecutive
# CI failures in July 2026. This script is the CI guard that prevents
# recurrence. Run locally before any MSRV-related commit; run in CI on
# every push.
#
# Usage: scripts/check-msrv.sh
# Exit: 0 if all sites agree, 1 otherwise.

set -euo pipefail
cd "$(dirname "$0")/.."

# --- Extract the canonical MSRV from Cargo.toml ---
CANONICAL=$(grep '^rust-version' Cargo.toml | head -1 | sed 's/.*"\([0-9.]*\)".*/\1/')
if [[ -z "$CANONICAL" ]]; then
  echo "FAIL: could not extract rust-version from Cargo.toml" >&2
  exit 1
fi
echo "Canonical MSRV (Cargo.toml): $CANONICAL"

FAILURES=0
check() {
  local location="$1" pattern="$2"
  if grep -q "$pattern" "$location" 2>/dev/null; then
    echo "  OK:   $location"
  else
    echo "FAIL:   $location — expected to contain '$pattern'" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

# --- ci.yml: matrix entry + msrv job ---
check ".github/workflows/ci.yml" "rust: \[stable, \"$CANONICAL\"\]"
check ".github/workflows/ci.yml" "Verify MSRV ($CANONICAL)"
check ".github/workflows/ci.yml" "toolchain: \"$CANONICAL\""

# --- flake.nix: msrv shell pin + header comment ---
check "flake.nix" "rust-bin.stable.\"${CANONICAL}.0\""
check "flake.nix" "MSRV ($CANONICAL)"

# --- docs/MSRV.md: headline ---
check "docs/MSRV.md" "\*\*$CANONICAL\*\*"

# --- Summary ---
if [[ "$FAILURES" -gt 0 ]]; then
  echo ""
  echo "MSRV DRIFT DETECTED: $FAILURES location(s) out of sync with Cargo.toml." >&2
  echo "Fix ALL of the above to match rust-version = \"$CANONICAL\" in Cargo.toml." >&2
  exit 1
fi
echo ""
echo "All MSRV locations agree on $CANONICAL."
