#!/usr/bin/env python3
"""A fake language server for the LSP client's tests (stdin/stdout JSON-RPC).

Flags (comma-separated) come from argv[1] or FAKE_LSP_MODE; what it did is appended to the file
argv[2] or FAKE_LSP_LOG, one line per event ("start <pid>", "close", "shutdown", "exit", ...).

  versionless  publishes without a version
  silent       publishes only for the first didOpen
  deaf         stops reading its input after `initialized`
  garbage      writes an unreadable frame after its first publish, then keeps running
  crash        exits on every didOpen
  tree         starts a `sleep` child (noted as "child <pid>") and never answers `shutdown`

Each didOpen is answered with one error whose message is "open=<documents open> text=<first
line>".
"""
import json
import os
import subprocess
import sys
import time

args = sys.argv[1:]
mode = set((args[0] if args and not args[0].startswith("-") else os.environ.get("FAKE_LSP_MODE", "")).split(","))
log = args[1] if len(args) > 1 else os.environ.get("FAKE_LSP_LOG", os.devnull)


def note(line):
    with open(log, "a") as f:
        f.write(line + "\n")


note("start %d" % os.getpid())
if "tree" in mode:
    child = subprocess.Popen(["sleep", "1000"])
    note("child %d" % child.pid)

stdin = sys.stdin.buffer
stdout = sys.stdout.buffer


def read():
    length = None
    while True:
        line = stdin.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        key, _, value = line.partition(b":")
        if key.strip().lower() == b"content-length":
            length = int(value)
    return json.loads(stdin.read(length))


def send(message):
    body = json.dumps(message).encode()
    stdout.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    stdout.flush()


opened = set()
opens = 0
while True:
    message = read()
    if message is None:
        note("eof")
        sys.exit(0)
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {}}})
    elif method == "initialized":
        if "deaf" in mode:
            note("deaf")
            time.sleep(1000)
    elif method == "textDocument/didOpen":
        opens += 1
        document = message["params"]["textDocument"]
        opened.add(document["uri"])
        if "crash" in mode:
            note("crash")
            os._exit(1)
        if "silent" in mode and opens > 1:
            continue
        lines = document["text"].splitlines()
        diagnostic = {
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "severity": 1,
            "message": "open=%d text=%s" % (len(opened), lines[0] if lines else ""),
        }
        params = {"uri": document["uri"], "diagnostics": [diagnostic]}
        if "versionless" not in mode:
            params["version"] = document["version"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params})
        if "garbage" in mode:
            stdout.write(b"garbage\r\n\r\n")
            stdout.flush()
    elif method == "textDocument/didClose":
        opened.discard(message["params"]["textDocument"]["uri"])
        note("close")
    elif method == "shutdown":
        note("shutdown")
        if "tree" not in mode:
            send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        note("exit")
        sys.exit(0)
