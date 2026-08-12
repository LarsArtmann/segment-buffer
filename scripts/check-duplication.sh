#!/usr/bin/env bash
# scripts/check-duplication.sh
#
# jscpd duplication gate. Scans src/ for copy-paste code blocks and fails
# if the duplication percentage exceeds the threshold defined in .jscpd.json.
#
# The gate targets library code (src/*.rs). The current baseline is ~1%
# (two intentional clones: encode_segment/encode_payload signatures in
# segment.rs and the AEAD output-assembly pattern in cipher.rs). The 2%
# threshold accommodates this baseline while catching significant new
# duplication.
#
# CI runs this via npm-installed jscpd@3. Locally, install jscpd and jq:
#   npm install -g jscpd@3
# The verify-gate skips this check if jscpd is not installed.

set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

if ! command -v jscpd &>/dev/null; then
	echo "SKIP: jscpd not installed (npm install -g jscpd@3)" >&2
	exit 0
fi

if ! command -v jq &>/dev/null; then
	echo "SKIP: jq not installed (required to parse jscpd JSON report)" >&2
	exit 0
fi

# Clean any stale report from a previous run.
rm -rf jscpd-report

jscpd src/ --config .jscpd.json

pct=$(jq '.statistics.total.percentage' jscpd-report/jscpd-report.json)
echo "Code duplication: ${pct}% (threshold: 2%)"

if [ "$(jq '.statistics.total.percentage > 2' jscpd-report/jscpd-report.json)" = "true" ]; then
	echo "ERROR: Code duplication ${pct}% exceeds 2% threshold" >&2
	echo "See jscpd-report/ for details." >&2
	exit 1
fi
