#!/usr/bin/env bash
# Drive the TUI under a pty: wait, type a prompt, wait for the answer, render the screen, then exit and keep the epilogue.
#   tui-session.sh <binary> <out-prefix> "<prompt>" [home] [answer-seconds]
set -u
BIN=$1; OUT=$2; PROMPT=$3; H=${4:-/home/ubuntu/anysola-code/build/home}; WAIT=${5:-30}
DIR="$(cd "$(dirname "$0")" && pwd)"
source "$DIR/env.sh" "$H"
cd "$HOME"
export TERM=xterm-256color COLORTERM=truecolor
RAW="$OUT.raw"
( sleep 10; printf '%s' "$PROMPT"; sleep 1; printf '\r'; sleep "$WAIT"; printf '/exit'; sleep 1; printf '\r'; sleep 4 ) |
  timeout $((WAIT + 30)) script -qfec "stty cols 120 rows 36; exec $BIN --standalone" "$RAW" >/dev/null 2>&1
"$DIR/../tools/venv/bin/python" - "$RAW" "$OUT" <<'PY'
import sys, pyte
raw, out = sys.argv[1], sys.argv[2]
data = open(raw, "rb").read()
cut = data.rfind(b"Script done")
if cut != -1: data = data[:cut]
if data.startswith(b"Script started"): data = data[data.find(b"\n") + 1:]
screen = pyte.Screen(120, 36); stream = pyte.ByteStream(screen)
best = None
# The last full frame before the TUI leaves the alternate screen (\x1b[?1049l) is the session screen.
leave = data.rfind(b"\x1b[?1049l")
session, after = (data[:leave], data[leave:]) if leave != -1 else (data, b"")
for i in range(0, len(session), 2048):
    stream.feed(session[i:i+2048])
    text = "\n".join(l.rstrip() for l in screen.display)
    # Keep the last frame before the exit command was typed.
    if text.strip() and "/exit" not in text: best = text
open(out + ".screen.txt", "w").write(best or "")
epi = pyte.Screen(120, 36); pyte.ByteStream(epi).feed(after)
open(out + ".exit.txt", "w").write("\n".join(l.rstrip() for l in epi.display if l.strip()))
PY
