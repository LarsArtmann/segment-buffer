#!/usr/bin/env bash
# scripts/check-cargo-lock-version.sh
#
# Asserts the `segment-buffer` version recorded in Cargo.lock matches the
# `version` field in Cargo.toml. The lock file drifts silently when a version
# bump in Cargo.toml is not followed by `cargo check` / `cargo build` — the
# v0.5.7 release failed on first `cargo publish` attempt for exactly this
# reason. This script catches the drift before a release tag is pushed.
#
# Usage: scripts/check-cargo-lock-version.sh
# Exit: 0 if versions agree, 1 on mismatch, 2 on extraction failure.

set -euo pipefail
cd "$(dirname "$0")/.."

# --- Extract the Cargo.toml version (canonical source) ---
cargo_version=$(grep -E '^version = ' Cargo.toml | head -1 | sed -E 's/^version = "([^"]+)".*/\1/')
if [[ -z "$cargo_version" ]]; then
	echo "FAIL: could not extract version from Cargo.toml" >&2
	exit 2
fi

# --- Extract the segment-buffer version from Cargo.lock ---
# Cargo.lock uses this format for the root crate:
#   name = "segment-buffer"
#   version = "0.6.0"
# We grab the version on the line immediately following the name match.
lock_version=$(grep -A1 '^name = "segment-buffer"$' Cargo.lock |
	tail -1 |
	sed -E 's/^version = "([^"]+)".*/\1/')

if [[ -z "$lock_version" ]]; then
	echo "FAIL: could not extract segment-buffer version from Cargo.lock" >&2
	exit 2
fi

# --- Compare ---
if [[ "$cargo_version" == "$lock_version" ]]; then
	echo "OK: Cargo.lock ($lock_version) == Cargo.toml ($cargo_version)"
	exit 0
fi

echo "FAIL: Cargo.lock version ($lock_version) != Cargo.toml version ($cargo_version)" >&2
echo "Fix: run \`cargo check\` or \`cargo build\` to sync Cargo.lock, then commit." >&2
exit 1
