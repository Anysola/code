#!/usr/bin/env bash
# Open the TUI, type a slash command (plus optional extra keys), render the screen.
#   tui-command.sh <binary> <out.txt> "/usage" [extra-keys] [home]
set -u
BIN=$1; OUT=$2; CMD=$3; EXTRA=${4:-}; H=${5:-/home/ubuntu/anysola-code/build/home}
DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/env.sh" "$H"
cd "$HOME"
export TERM=xterm-256color COLORTERM=truecolor
( sleep 10; printf '%s' "$CMD"; sleep 1; printf '\r'; sleep 4; [ -n "$EXTRA" ] && printf '%s' "$EXTRA"; sleep 4 ) |
  timeout 30 script -qfec "stty cols 120 rows 36; exec $BIN ${TUI_ARGS---standalone}" "$OUT.raw" >/dev/null 2>&1
"$DIR/../tools/venv/bin/python" - "$OUT.raw" > "$OUT" <<'PY'
import sys, pyte
data = open(sys.argv[1], "rb").read()
cut = data.rfind(b"Script done")
if cut != -1: data = data[:cut]
if data.startswith(b"Script started"): data = data[data.find(b"\n") + 1:]
leave = data.rfind(b"\x1b[?1049l")
if leave != -1: data = data[:leave]
screen = pyte.Screen(120, 36); stream = pyte.ByteStream(screen); best = ""
for i in range(0, len(data), 2048):
    stream.feed(data[i:i+2048])
    text = "\n".join(l.rstrip() for l in screen.display)
    if text.strip(): best = text
print("\n".join(l for l in best.split("\n") if l.strip()))
PY
rm -f "$OUT.raw"
