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
# CI runs this via pnpm-installed jscpd@3. Locally, install jscpd and jq:
#   pnpm add -g jscpd@3
# The verify-gate skips this check if jscpd is not installed.
#
# KNOWN GOTCHA: jscpd v3's --exitCode flag is broken. It always exits 0 even
# when duplication is found, so DO NOT "simplify" the jq threshold check at
# the bottom into a bare `jscpd --exitCode 2` call — that silently disables
# the gate. The jq check below is the actual enforcement.
#
# KNOWN GOTCHA: jscpd behaves differently with CLI flags vs the config file.
# Running `jscpd src/ --format rust --min-lines 5 --min-tokens 60` reports
# 0 clones, while the equivalent settings in .jscpd.json report the real
# ~1% baseline (2 intentional clones: the segment encode signatures and the
# AEAD output assembly). The tokenizer/format resolution differs between the
# two invocation paths. The 2% threshold is calibrated against CONFIG-FILE
# behavior, so ALWAYS run jscpd the way this script does (--config
# .jscpd.json). Do not trust a raw-CLI-flag run's 0% as evidence that the
# gate is unnecessary.

set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

if ! command -v jscpd &>/dev/null; then
	echo "SKIP: jscpd not installed (pnpm add -g jscpd@3)" >&2
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
