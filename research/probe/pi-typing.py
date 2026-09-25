#!/usr/bin/env python3
"""Where pi's caret is while the operator types a line longer than the pane is wide.

The phone report: typing into pi, the view does not follow the text sideways, where it does in
Claude. The client follows `pane.cursor`; this logs, off a real `terminal session observe` stream
through vt.py, where that cursor is and whether it is shown, next to the row the text lands on.

Runs in a throwaway named herdr session, torn down at the end (#97).
"""
import os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import importlib
sel = importlib.import_module("pi-selector")
import vt

sel.NAME = f"kampr-probe-pitype-{os.getpid()}"
sel.SOCK = os.path.join(sel.HOME, "herdr", "sessions", sel.NAME, "herdr.sock")
sel.COLS, sel.ROWS = 100, 70
HARNESS = sys.argv[1] if len(sys.argv) > 1 else "pi"


class Shown(sel.Watch):
    """The last DECTCEM the harness wrote: `?25h` shows the caret, `?25l` hides it."""

    def pump(self):
        import base64, json
        self.shown = None
        for line in self.child.stdout:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if rec.get("type") != "terminal.frame":
                continue
            data = base64.b64decode(rec["bytes"]).decode("utf-8", "replace")
            on, off = data.rfind("\x1b[?25h"), data.rfind("\x1b[?25l")
            if on != off:
                self.shown = on > off
            with self.lock:
                vt.feed(self.screen, data)


def main():
    sel.start()
    try:
        pane = sel.call("workspace.create", {"label": "p", "cwd": "/tmp"})["root_pane"]["pane_id"]
        watch = Shown(pane)
        sel.send(pane, f"{HARNESS}\n")
        time.sleep(10)
        sel.report(watch, "idle")
        text = "the quick brown fox jumps over the lazy dog " * 3
        for n, ch in enumerate(text):
            sel.send(pane, ch)
            time.sleep(0.03)
            if n % 20 == 19:
                time.sleep(0.4)
                rows, x, y = watch.look()
                shown = watch.shown
                hit = [i for i, r in enumerate(rows) if "lazy" in r or "fox" in r]
                print(f"typed {n+1:3d}: caret col {x:3d} row {y:2d} shown={shown} text rows {hit}")
        time.sleep(0.5)
        sel.report(watch, "typed")
        watch.close()
    finally:
        sel.stop()


if __name__ == "__main__":
    main()
