#!/usr/bin/env python3
# Drive the TUI under a pty, take colored PNG (and text) snapshots of pyte's screen.
#   tui-shot.py BIN OUT_PREFIX [--cols 120] [--rows 36] [--home DIR] [--light] [--args "..."] STEP...
# Steps: wait:SECS  type:TEXT  key:enter|esc|tab|stab|up|down|ctrlc|ctrlp  shot:NAME  shotmain:NAME (screen after exit)
# --light answers the terminal's OSC 11 background query with a light colour (light theme mode).
import os, sys, pty, time, select, signal, struct, fcntl, termios, argparse, re
import pyte
from PIL import Image, ImageDraw, ImageFont

ap = argparse.ArgumentParser()
ap.add_argument("bin"); ap.add_argument("out")
ap.add_argument("--cols", type=int, default=120); ap.add_argument("--rows", type=int, default=36)
ap.add_argument("--home", default="/home/ubuntu/anysola-code/build/home")
ap.add_argument("--cwd", default=None)
ap.add_argument("--light", action="store_true")
ap.add_argument("--args", default="--standalone")
ap.add_argument("steps", nargs="*")
a = ap.parse_intermixed_args()

KEYS = {"enter": b"\r", "esc": b"\x1b", "tab": b"\t", "stab": b"\x1b[Z", "up": b"\x1b[A", "down": b"\x1b[B",
        "ctrlc": b"\x03", "ctrlp": b"\x10", "ctrld": b"\x04", "bs": b"\x7f"}

env = dict(os.environ)
H = a.home
env.update(HOME=H, XDG_CONFIG_HOME=f"{H}/.config", XDG_DATA_HOME=f"{H}/.local/share", XDG_CACHE_HOME=f"{H}/.cache",
           XDG_STATE_HOME=f"{H}/.local/state", OPENCODE_DISABLE_AUTOUPDATE="1", TERM="xterm-256color", COLORTERM="truecolor")
pid, fd = pty.fork()
if pid == 0:
    os.chdir(a.cwd or H)
    os.execvpe(a.bin, [a.bin] + a.args.split(), env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
os.kill(pid, signal.SIGWINCH)

screen = pyte.Screen(a.cols, a.rows)
stream = pyte.ByteStream(screen)
raw = bytearray()
BG_DARK, FG_DARK = (0x11, 0x0f, 0x0d), (0xf8, 0xf6, 0xf4)
BG_LIGHT, FG_LIGHT = (0xfb, 0xfa, 0xf8), (0x1a, 0x18, 0x15)
termbg, termfg = (BG_LIGHT, FG_LIGHT) if a.light else (BG_DARK, FG_DARK)

def reply(chunk):
    out = b""
    # OSC 10/11 colour queries
    for m in re.finditer(rb"\x1b\](1[01]);\?(\x07|\x1b\\)", chunk):
        c = termbg if m.group(1) == b"11" else termfg
        out += b"\x1b]" + m.group(1) + b";rgb:%02x%02x/%02x%02x/%02x%02x" % (c[0], c[0], c[1], c[1], c[2], c[2]) + m.group(2)
    if b"\x1b[c" in chunk or b"\x1b[0c" in chunk: out += b"\x1b[?62;22c"
    if b"\x1b[6n" in chunk: out += b"\x1b[1;1R"
    if out: os.write(fd, out)

def pump(secs):
    end = time.time() + secs
    while True:
        left = end - time.time()
        if left <= 0: return
        r, _, _ = select.select([fd], [], [], min(left, 0.05))
        if fd in r:
            try: chunk = os.read(fd, 65536)
            except OSError: return
            if not chunk: return
            reply(chunk)
            raw.extend(chunk)
            stream.feed(chunk)

FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FONTB = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SIZE = 16
font, fontb = ImageFont.truetype(FONT, SIZE), ImageFont.truetype(FONTB, SIZE)
CW, CH = 10, 20
NAMED = {"black": (0, 0, 0), "red": (205, 49, 49), "green": (13, 188, 121), "brown": (229, 229, 16), "yellow": (229, 229, 16),
         "blue": (36, 114, 200), "magenta": (188, 63, 188), "cyan": (17, 168, 205), "white": (229, 229, 229)}

def color(c, default):
    if c == "default": return default
    if c in NAMED: return NAMED[c]
    if re.fullmatch(r"[0-9a-fA-F]{6}", c): return tuple(int(c[i:i+2], 16) for i in (0, 2, 4))
    return default

# Block elements drawn as rectangles: (x0, y0, x1, y1) in cell fractions
BLOCKS = {"█": [(0, 0, 1, 1)], "▀": [(0, 0, 1, .5)], "▄": [(0, .5, 1, 1)], "▌": [(0, 0, .5, 1)], "▐": [(.5, 0, 1, 1)],
          "▘": [(0, 0, .5, .5)], "▝": [(.5, 0, 1, .5)], "▖": [(0, .5, .5, 1)], "▗": [(.5, .5, 1, 1)],
          "▚": [(0, 0, .5, .5), (.5, .5, 1, 1)], "▞": [(.5, 0, 1, .5), (0, .5, .5, 1)],
          "▛": [(0, 0, 1, .5), (0, .5, .5, 1)], "▜": [(0, 0, 1, .5), (.5, .5, 1, 1)],
          "▙": [(0, 0, .5, .5), (0, .5, 1, 1)], "▟": [(.5, 0, 1, .5), (0, .5, 1, 1)],
          "▔": [(0, 0, 1, .125)], "▁": [(0, .875, 1, 1)], "▂": [(0, .75, 1, 1)], "▃": [(0, .625, 1, 1)],
          "▅": [(0, .375, 1, 1)], "▆": [(0, .25, 1, 1)], "▇": [(0, .125, 1, 1)],
          "▏": [(0, 0, .125, 1)], "▎": [(0, 0, .25, 1)], "▍": [(0, 0, .375, 1)], "▊": [(0, 0, .75, 1)], "▉": [(0, 0, .875, 1)],
          "▕": [(.875, 0, 1, 1)], "┃": [(.4, 0, .6, 1)], "╹": [(.4, 0, .6, .5)], "╻": [(.4, .5, .6, 1)]}

def shot(name, scr=None):
    screen_ = scr or screen
    img = Image.new("RGB", (a.cols * CW, a.rows * CH), termbg)
    d = ImageDraw.Draw(img)
    lines = []
    for y in range(a.rows):
        row = screen_.buffer[y]; text = ""
        for x in range(a.cols):
            ch = row[x]
            fg, bg = color(ch.fg, termfg), color(ch.bg, termbg)
            if ch.reverse: fg, bg = bg, fg
            x0, y0 = x * CW, y * CH
            d.rectangle([x0, y0, x0 + CW - 1, y0 + CH - 1], fill=bg)
            c = ch.data or " "
            text += c
            if c in BLOCKS:
                for (fx0, fy0, fx1, fy1) in BLOCKS[c]:
                    d.rectangle([x0 + fx0 * CW, y0 + fy0 * CH, x0 + fx1 * CW - 1 + (1 if fx1 == 1 else 0), y0 + fy1 * CH - 1 + (1 if fy1 == 1 else 0)], fill=fg)
            elif c.strip():
                d.text((x0, y0 + 1), c, font=fontb if ch.bold else font, fill=fg)
        lines.append(text.rstrip())
    img.save(f"{a.out}.{name}.png")
    open(f"{a.out}.{name}.txt", "w").write("\n".join(lines))
    print("shot", f"{a.out}.{name}.png")

for step in a.steps:
    kind, _, arg = step.partition(":")
    if kind == "wait": pump(float(arg))
    elif kind == "type":
        for ch in arg:
            os.write(fd, ch.encode()); pump(0.03)
    elif kind == "key": os.write(fd, KEYS[arg]); pump(0.2)
    elif kind == "shot": pump(0.3); shot(arg)
    elif kind == "shotmain":
        # The main screen after the TUI leaves the alternate screen (pyte has no alternate buffer).
        pump(0.3)
        cut = raw.rfind(b"\x1b[?1049l")
        main = pyte.Screen(a.cols, a.rows); pyte.ByteStream(main).feed(bytes(raw[cut + 8:] if cut != -1 else raw))
        shot(arg, main)
try:
    os.kill(pid, signal.SIGTERM); pump(1)
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError: pass
