#!/usr/bin/env bash
# scripts/check-changelog-links.sh
#
# Validates that every version reference in CHANGELOG.md points to a real
# GitHub tag. Catches the recurring drift where a CHANGELOG entry references
# a tag that hasn't been pushed yet (or was renamed).
#
# Usage: scripts/check-changelog-links.sh
# Exits non-zero if any link is broken.

set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

CHANGELOG="CHANGELOG.md"

if [[ ! -f "$CHANGELOG" ]]; then
    echo "ERROR: $CHANGELOG not found" >&2
    exit 1
fi

# Extract GitHub compare/tag URLs from CHANGELOG.md
# Matches patterns like:
#   https://github.com/LarsArtmann/segment-buffer/compare/v0.5.3...v0.5.4
#   https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.5.4
MAPFILE -t URLS < <(
    grep -oE 'https://github\.com/LarsArtmann/segment-buffer/(compare|releases/tag)/[^"<>[:space:]]+' "$CHANGELOG" || true
)

if [[ ${#URLS[@]} -eq 0 ]]; then
    echo "No GitHub URLs found in $CHANGELOG"
    exit 0
fi

PASS=0
FAIL=0

for url in "${URLS[@]}"; do
    # Use the GitHub API to check if the tag(s) exist
    # For compare URLs, extract both tags
    if [[ "$url" == *"/compare/"* ]]; then
        # Extract tags from compare URL: .../compare/vA...vB
        tags="${url#*/compare/}"
        tag_a="${tags%%...*}"
        tag_b="${tags##*...}"
        for tag in "$tag_a" "$tag_b"; do
            http_code=$(curl -sSL -o /dev/null -w '%{http_code}' \
                "https://api.github.com/repos/LarsArtmann/segment-buffer/git/ref/tags/$tag" 2>/dev/null || echo "000")
            if [[ "$http_code" == "200" ]]; then
                PASS=$((PASS + 1))
            else
                echo "FAIL: tag '$tag' not found (HTTP $http_code) — referenced in $url"
                FAIL=$((FAIL + 1))
            fi
        done
    elif [[ "$url" == *"/releases/tag/"* ]]; then
        tag="${url##*/releases/tag/}"
        http_code=$(curl -sSL -o /dev/null -w '%{http_code}' \
            "https://api.github.com/repos/LarsArtmann/segment-buffer/git/ref/tags/$tag" 2>/dev/null || echo "000")
        if [[ "$http_code" == "200" ]]; then
            PASS=$((PASS + 1))
        else
            echo "FAIL: tag '$tag' not found (HTTP $http_code) — referenced in $url"
            FAIL=$((FAIL + 1))
        fi
    fi
done

echo ""
echo "CHANGELOG link check: $PASS passed, $FAIL failed"

if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
