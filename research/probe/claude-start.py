#!/usr/bin/env python3
"""Where the caret and the end of the record sit while `claude` starts in a directory it has never
been trusted in: the trust prompt, the answer, and the screen it draws after.

The phone report: launching Claude in a new directory loses the bottom of the screen, and switching
away and back brings it back. This logs every change in (caret row, last non-blank row) off a real
`terminal session observe` stream through vt.py — the grid the node's emulator builds — so the
client's band can be computed against the measured rows.

Runs in a throwaway named herdr session with every CLAUDE* variable scrubbed (an inherited
CLAUDE_CODE_CHILD_SESSION suppresses the prompt), torn down at the end (#97).
"""
import os, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import importlib
sel = importlib.import_module("pi-selector")

sel.NAME = f"kampr-probe-cstart-{os.getpid()}"
sel.SOCK = os.path.join(sel.HOME, "herdr", "sessions", sel.NAME, "herdr.sock")
sel.COLS, sel.ROWS = int(os.environ.get("COLS", 120)), int(os.environ.get("ROWS", 40))

for k in [k for k in os.environ if k.startswith("CLAUDE") and k != "CLAUDE_CODE_DISABLE_MOUSE_CLICKS"]:
    del os.environ[k]


def timeline(watch, seconds, t0, last):
    end = time.time() + seconds
    while time.time() < end:
        rows, x, y = watch.look()
        filled = sel.nonblank(rows)
        reading = (y, filled[-1] if filled else -1)
        if reading != last:
            print(f"  t={time.time() - t0:6.2f}s caret row {reading[0]:2d}  last content row {reading[1]:2d}"
                  f"  below caret {reading[1] - reading[0]:3d}")
            last = reading
        time.sleep(0.02)
    return last


def main():
    cwd = tempfile.mkdtemp(prefix="kampr-cstart-", dir="/var/tmp")
    sel.start()
    try:
        pane = sel.call("workspace.create", {"label": "c", "cwd": cwd})["root_pane"]["pane_id"]
        watch = sel.Watch(pane)
        time.sleep(1.0)
        t0 = time.time()
        last = None
        print("-- typed `claude`")
        sel.send(pane, "claude\n")
        last = timeline(watch, 8, t0, last)
        sel.report(watch, "trust prompt")
        print("-- down, enter")
        sel.send(pane, "\x1b[B")
        time.sleep(0.3)
        sel.send(pane, "\r")
        last = timeline(watch, 12, t0, last)
        sel.report(watch, "after trust")
        watch.close()
    finally:
        sel.stop()


if __name__ == "__main__":
    main()
