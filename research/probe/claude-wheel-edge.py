#!/usr/bin/env python3
"""What Claude Code does with SGR wheel reports once its transcript can scroll no further, at the
top and at the bottom: whether the composer starts walking its prompt history.

    VENV/bin/python research/probe/claude-wheel-edge.py [fullscreen|default ...]

Same mock endpoint and throwaway config as claude-render.py. Two turns are submitted so the
composer has a history to recall, the second with a long reply so the transcript scrolls.
`KWE_NOCLICKS=1` runs it with `CLAUDE_CODE_DISABLE_MOUSE_CLICKS=true`, as the operator's shell does.
Needs pyte (claude-replay.py says how).
"""

import http.server
import importlib.util
import os
import shutil
import sys
import tempfile
import threading
import time

import pyte

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
spec = importlib.util.spec_from_file_location("claude_render", os.path.join(HERE, "claude-render.py"))
cr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cr)


def wheel(up, n):
    return ("\x1b[<%d;10;10M" % (64 if up else 65)) * n


def lines(c):
    sc = pyte.Screen(cr.COLS, cr.ROWS)
    pyte.ByteStream(sc).feed(c.since(0))
    return [r.rstrip() for r in sc.display]


def composer(c):
    rows = [r for r in lines(c) if r.strip()]
    prompt = [r.strip() for r in rows if r.lstrip().startswith("❯")]
    return (prompt[-1:] or ["?"]) + [f"top: {rows[0].strip()[:50]}" if rows else "top: -"]


def step(c, label, data, settle=1.5):
    os.write(c.fd, data.encode())
    c.pump(settle)
    print(f"  {label:<34} composer: {composer(c)}")


def run(tui, port):
    work = tempfile.mkdtemp(prefix="kwe-", dir=os.environ.get("XDG_RUNTIME_DIR"))
    c = cr.Claude(tui, port, work)
    try:
        c.pump(12, until=b"\xe2\x9d\xaf")
        c.pump(2)
        print(f"=== tui={tui}")
        for text in ("FIRSTPROMPT short", "SECONDPROMPT LONGANSWER"):
            c.type(text)
            c.pump(0.5)
            c.type("\r")
            c.pump(30, until=b"REPLYEND" if "LONG" in text else b"Short answer")
            c.pump(2)
        print(f"  after two turns                    composer: {composer(c)}")
        for n in (1, 5, 40, 40, 40):
            step(c, f"wheel up x{n}", wheel(True, n))
        for n in (40, 40, 40, 40, 1, 1, 5):
            step(c, f"wheel down x{n}", wheel(False, n))
        step(c, "cursor up (ESC O A) x1", "\x1bOA")
    finally:
        c.close()
        shutil.rmtree(work, ignore_errors=True)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), cr.Mock)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    for tui in sys.argv[1:] or ["fullscreen", "default"]:
        run(tui, server.server_address[1])
    server.shutdown()


if __name__ == "__main__":
    main()
