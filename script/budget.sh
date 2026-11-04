#!/usr/bin/env bash
# Builds the release binary and fails when it breaks the budgets of ARCHITECTURE.md:
# stripped size <= 25 MB, `anysola --version` <= 15 ms wall and <= 10 MB max RSS (best of 5;
# wall time from `date +%s%N`, since /usr/bin/time rounds to 10 ms).
set -euo pipefail
cd "$(dirname "$0")/.."

MAX_BYTES=$((25 * 1024 * 1024))
MAX_MS=15
MAX_RSS_KB=$((10 * 1024))

cargo build --release --locked -p anysola
BIN=target/release/anysola
[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

size=$(wc -c < "$BIN")
best_ms=""
best_rss=""
for _ in 1 2 3 4 5; do
  start=$(date +%s%N)
  "$BIN" --version >/dev/null
  ms=$(( ($(date +%s%N) - start) / 1000000 ))
  rss=$(/usr/bin/time -f "%M" "$BIN" --version 2>&1 >/dev/null)
  if [ -z "$best_ms" ] || [ "$ms" -lt "$best_ms" ]; then best_ms=$ms; fi
  if [ -z "$best_rss" ] || [ "$rss" -lt "$best_rss" ]; then best_rss=$rss; fi
done

printf 'binary   %d bytes (%.1f MB, budget 25 MB)\n' "$size" "$(awk "BEGIN{print $size/1048576}")"
printf 'version  %d ms (budget %d ms), %d KB max RSS (budget %d KB)\n' "$best_ms" "$MAX_MS" "$best_rss" "$MAX_RSS_KB"

fail=0
[ "$size" -le "$MAX_BYTES" ] || { echo "FAIL: binary over 25 MB" >&2; fail=1; }
[ "$best_ms" -le "$MAX_MS" ] || { echo "FAIL: --version over ${MAX_MS} ms" >&2; fail=1; }
[ "$best_rss" -le "$MAX_RSS_KB" ] || { echo "FAIL: --version over 10 MB RSS" >&2; fail=1; }
exit "$fail"
