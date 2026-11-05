#!/usr/bin/env python3
# Starts the TUI under a pseudo-terminal, reports the time to its first frame (the home prompt
# drawn) and its resident memory after it settles, then exits it with ctrl+c.
#   tui-idle.py BIN  (environment: HOME, ANYSOLA_API_URL, ANYSOLA_TOKEN as for the binary)
import fcntl, os, pty, select, struct, sys, termios, time

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv(sys.argv[1], [sys.argv[1]])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 0, 0))
start = time.monotonic()
seen, first = b"", None
deadline = start + 10
while time.monotonic() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.05)
    if ready:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        seen += chunk
        # Answer the terminal queries a real emulator answers (device attributes, background).
        if b"\x1b[c" in chunk:
            os.write(fd, b"\x1b[?62c")
        if b"\x1b]11;?" in chunk:
            os.write(fd, b"\x1b]11;rgb:1111/0f0f/0d0d\x07")
        if first is None and b"Ask anything" in seen:
            first = time.monotonic() - start
            deadline = time.monotonic() + 3
rss = 0
try:
    with open(f"/proc/{pid}/status") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                rss = int(line.split()[1])
except OSError:
    print("-1 0")
    sys.exit(0)
os.write(fd, b"\x03")
time.sleep(0.5)
os.kill(pid, 9) if os.waitpid(pid, os.WNOHANG) == (0, 0) else None
print(f"{-1 if first is None else int(first * 1000)} {rss}")
