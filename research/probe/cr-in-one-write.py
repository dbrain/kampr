#!/usr/bin/env python3
"""#79 again, on the Claude in use now: does a carriage return that arrives in the same write as
the text submit a ready Claude, or become a newline in its box?

A booted, trusted `claude` in a throwaway named herdr session (#97). Each trial sends a marked
line either as one `pane.send_text` of `text + "\r"` or as the composer sends it, text then `\r`
as a second call, and reads the screen: a submit puts the marker above the box's top rule, and a
held line leaves it inside the box with the caret on a second row.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-cr-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
RULE = "─" * 20


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def box(rows):
    rules = [i for i, r in enumerate(rows) if r.startswith(RULE)]
    return (rules[-2], rules[-1]) if len(rules) >= 2 else (None, None)


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-cr-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        time.sleep(12)
        if any("trust" in r.lower() for r in watch.look()[0]):
            send(pane, "1")
            time.sleep(6)
        for n, how in enumerate(["one", "two"] * 3):
            marker = f"CR{n:02d}{how.upper()}"
            text = f"{marker} reply with only the word ok"
            if how == "one":
                send(pane, text + "\r")
            else:
                send(pane, text)
                send(pane, "\r")
            time.sleep(3)
            rows, x, y = watch.look()
            top, bottom = box(rows)
            above = top is not None and any(marker in r for r in rows[:top])
            inside = [r for r in rows[top + 1:bottom]] if top is not None else []
            print(f"  {how:3s} write(s): {'SUBMIT' if above else 'HELD'}  box rows={len(inside)}  caret row={y}")
            for r in inside:
                print(f"      | {r[:90]!r}")
            if above:
                time.sleep(6)
            else:
                send(pane, "\x03")
                time.sleep(1)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
