#!/usr/bin/env bash
# scripts/check-html-root-url.sh
#
# Asserts the version pinned in `#![doc(html_root_url = "...")]` in src/lib.rs
# matches the `version` field in Cargo.toml. The html_root_url pin rots silently
# on every release otherwise (broken intra-doc links on docs.rs).
#
# Usage: scripts/check-html-root-url.sh
# Exits non-zero on mismatch.

set -eu

cd "$(dirname "$0")/.." || exit 1

cargo_version=$(grep -E '^version = ' Cargo.toml | head -1 | sed -E 's/^version = "([^"]+)".*/\1/')

url_version=$(grep -E 'html_root_url = "https://docs\.rs/segment-buffer/' src/lib.rs \
  | head -1 \
  | sed -E 's|.*/segment-buffer/([^"]+)".*|\1|')

if [[ -z "$cargo_version" || -z "$url_version" ]]; then
  echo "FAIL: could not extract one of the versions." >&2
  echo "  Cargo.toml version:    '${cargo_version:-<not found>}'" >&2
  echo "  html_root_url version: '${url_version:-<not found>}'" >&2
  exit 2
fi

if [[ "$cargo_version" == "$url_version" ]]; then
  echo "OK: html_root_url ($url_version) == Cargo.toml version ($cargo_version)"
  exit 0
fi

echo "FAIL: html_root_url version ($url_version) != Cargo.toml version ($cargo_version)" >&2
echo "Fix: edit src/lib.rs and update the URL in:" >&2
echo '  #![doc(html_root_url = "https://docs.rs/segment-buffer/<version>")]' >&2
exit 1
