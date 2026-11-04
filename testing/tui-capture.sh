#!/usr/bin/env bash
# Start the TUI under a pseudo-terminal, let it draw, and render the first screen as text.
#   tui-capture.sh <binary> <out.txt> [home] [seconds] [cols] [rows]
set -u
BIN=$1; OUT=$2; H=${3:-/home/ubuntu/anysola-code/build/home}; SECS=${4:-15}; COLS=${5:-120}; ROWS=${6:-36}
DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/env.sh" "$H"
RAW="$OUT.raw"
cd "$HOME"
export TERM=xterm-256color COLORTERM=truecolor
( sleep "$SECS" ) | timeout $((SECS + 5)) script -qfec "stty cols $COLS rows $ROWS; exec $BIN --standalone" "$RAW" >/dev/null 2>&1
"$DIR/../tools/venv/bin/python" - "$RAW" "$COLS" "$ROWS" > "$OUT" <<'PY'
import sys, pyte
raw, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
screen = pyte.Screen(cols, rows); stream = pyte.ByteStream(screen)
best = None
data = open(raw, "rb").read()
cut = data.rfind(b"Script done")
if cut != -1: data = data[:cut]
if data.startswith(b"Script started"): data = data[data.find(b"\n") + 1:]
for i in range(0, len(data), 1024):
    stream.feed(data[i:i+1024])
    text = "\n".join(l.rstrip() for l in screen.display)
    if text.strip(): best = text
print(best or "")
PY
