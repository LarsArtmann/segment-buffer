#!/usr/bin/env bash
# Sweep all zstd compression levels (1-22) across all payload kinds.
# Outputs a TSV table for easy analysis.
set -euo pipefail

cd "$(dirname "$0")/.."

COUNT=1000000
BATCH=5000
OUT="docs/perf/2026-08-10_compression-level-sweep.tsv"

printf "level\tkind\tload_ips\tload_mibs\tcomp_ratio\tpeak_disk_mib\tdrain_ips\tdrain_mibs\tload_p99_us\tdrain_p99_us\n" >"$OUT"

for kind in uniform text json random; do
	for level in $(seq 1 22); do
		echo "=== zstd-$level / $kind ===" >&2
		output=$(cargo run --release --example scaling -- "$COUNT" "$BATCH" "$level" 10 "$kind" 2>/dev/null)

		# Parse: two phases (load then drain), each has items/sec, MiB/s, latency.
		# load phase is first, drain phase is second.
		load_ips=$(echo "$output" | grep '^items/sec:' | sed -n '1p' | awk '{print $2}')
		load_mibs=$(echo "$output" | grep '^MiB/s:' | sed -n '1p' | awk '{print $2}')
		comp_ratio=$(echo "$output" | grep '^comp ratio:' | awk '{print $3}')
		peak_disk=$(echo "$output" | grep '^peak disk:' | awk '{print $3}')
		drain_ips=$(echo "$output" | grep '^items/sec:' | sed -n '2p' | awk '{print $2}')
		drain_mibs=$(echo "$output" | grep '^MiB/s:' | sed -n '2p' | awk '{print $2}')
		load_p99=$(echo "$output" | grep 'p99=' | sed -n '1p' | sed 's/.*p99=//' | awk '{print $1}')
		drain_p99=$(echo "$output" | grep 'p99=' | sed -n '2p' | sed 's/.*p99=//' | awk '{print $1}')

		printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
			"$level" "$kind" "$load_ips" "$load_mibs" "$comp_ratio" "$peak_disk" \
			"$drain_ips" "$drain_mibs" "$load_p99" "$drain_p99" >>"$OUT"
	done
done

echo "Done. Results in $OUT"
