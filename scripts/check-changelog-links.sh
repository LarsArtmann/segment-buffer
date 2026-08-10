#!/usr/bin/env bash
# scripts/check-changelog-links.sh
#
# Validates that every version reference in CHANGELOG.md points to a real
# GitHub tag. Catches the recurring drift where a CHANGELOG entry references
# a tag that hasn't been pushed yet (or was renamed).
#
# Usage: scripts/check-changelog-links.sh
#        GITHUB_TOKEN=... scripts/check-changelog-links.sh
#
# Exits non-zero if any link is broken. Exits zero (with a warning) if the
# GitHub API rate limit is exhausted — this is an infrastructure issue, not
# a broken link, so it degrades gracefully instead of failing CI.
#
# Rate limits:
#   Unauthenticated:  60 requests/hour per IP
#   With GITHUB_TOKEN: 5000 requests/hour per token
#
# CI automatically provides GITHUB_TOKEN via secrets.GITHUB_TOKEN. For local
# runs, set it manually if you hit the 60/hour limit.

set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

CHANGELOG="CHANGELOG.md"

if [[ ! -f "$CHANGELOG" ]]; then
	echo "ERROR: $CHANGELOG not found" >&2
	exit 1
fi

# Build curl auth args if GITHUB_TOKEN is set.
AUTH_ARGS=()
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
	AUTH_ARGS+=(-H "Authorization: Bearer $GITHUB_TOKEN")
fi

# Extract GitHub compare/tag URLs from CHANGELOG.md
# Matches patterns like:
#   https://github.com/LarsArtmann/segment-buffer/compare/v0.5.3...v0.5.4
#   https://github.com/LarsArtmann/segment-buffer/releases/tag/v0.5.4
mapfile -t URLS < <(
	grep -oE 'https://github\.com/LarsArtmann/segment-buffer/(compare|releases/tag)/[^"<>[:space:]]+' "$CHANGELOG" || true
)

if [[ ${#URLS[@]} -eq 0 ]]; then
	echo "No GitHub URLs found in $CHANGELOG"
	exit 0
fi

PASS=0
FAIL=0

# check_tag <tag> <source_url>
# Queries the GitHub git/ref API. Returns 0 on success, 1 on failure.
# On HTTP 403 (rate limit), warns and exits the script with 0 (graceful
# degradation) — a rate limit is not a broken link.
check_tag() {
	local tag="$1"
	local source_url="$2"
	local http_code
	http_code=$(curl -sSL -o /dev/null -w '%{http_code}' \
		"${AUTH_ARGS[@]}" \
		"https://api.github.com/repos/LarsArtmann/segment-buffer/git/ref/tags/$tag" \
		2>/dev/null || echo "000")

	case "$http_code" in
		200)
			PASS=$((PASS + 1))
			;;
		403)
			# Rate-limited. Degrade gracefully — this is an infrastructure
			# issue, not a broken link. The link will be checked on the next
			# run with a fresh budget.
			echo "WARN: GitHub API returned 403 (rate limit likely exhausted)." >&2
			if [[ -z "${GITHUB_TOKEN:-}" ]]; then
				echo "  Tip: set GITHUB_TOKEN to bump from 60/hr to 5000/hr." >&2
			fi
			echo "  Skipping remaining tag checks." >&2
			echo ""
			echo "CHANGELOG link check: $PASS passed, $FAIL failed (rate-limited, degraded)"
			exit 0
			;;
		*)
			echo "FAIL: tag '$tag' not found (HTTP $http_code) — referenced in $source_url"
			FAIL=$((FAIL + 1))
			;;
	esac
}

for url in "${URLS[@]}"; do
	# Use the GitHub API to check if the tag(s) exist
	# For compare URLs, extract both tags
	if [[ "$url" == *"/compare/"* ]]; then
		# Extract tags from compare URL: .../compare/vA...vB
		tags="${url#*/compare/}"
		tag_a="${tags%%...*}"
		tag_b="${tags##*...}"
		for tag in "$tag_a" "$tag_b"; do
			# Skip HEAD — the Keep-a-Changelog convention for the [Unreleased]
			# compare link. It resolves to the default-branch tip on GitHub, not
			# a tag, so the git/ref/tags API correctly 404s.
			if [[ "$tag" == "HEAD" ]]; then
				continue
			fi
			check_tag "$tag" "$url"
		done
	elif [[ "$url" == *"/releases/tag/"* ]]; then
		tag="${url##*/releases/tag/}"
		check_tag "$tag" "$url"
	fi
done

echo ""
echo "CHANGELOG link check: $PASS passed, $FAIL failed"

if [[ "$FAIL" -gt 0 ]]; then
	exit 1
fi
