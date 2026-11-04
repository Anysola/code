#!/usr/bin/env bash
# Regenerates spec/verbatim/tools/shell-scan-corpus.jsonl.gz from the 0.5.1 fork: records every input
# its shell scanner tests feed the scanner, then runs ShellParse.scan over them.
# Test failures (timing limits, real-shell oracles) do not matter here; only the inputs do.
# Usage: script/shell-corpus.sh <path to the 0.5.1 fork checkout>
set -euo pipefail
fork=$(cd "$1" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tail -n +2 "$fork/packages/core/src/shell/scan.ts" > "$work/scan.ts"
(cd "$fork/packages/core" && RECORD_SCAN="$work/scan.ts" RECORD_FORK="$fork" RECORD_OUT="$work/recorded.jsonl" \
  bun test --preload "$root/script/shell-corpus/record.ts" test/shell-scan test/shell-parse >/dev/null 2>&1 || true)
sort -u "$work/recorded.jsonl" > "$work/inputs.jsonl"
env -i HOME=/home/tester USER=tester PROJECTS=/srv/projects "$(command -v bun)" "$root/script/shell-corpus/generate.ts" "$fork" "$work/inputs.jsonl" "$work/corpus.jsonl"
gzip -9n < "$work/corpus.jsonl" > "$root/spec/verbatim/tools/shell-scan-corpus.jsonl.gz"
