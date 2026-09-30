#!/usr/bin/env python3
"""Where Claude's composer wraps typed text, what sits between the wrap and the right edge, and
where the caret goes when it wraps.

Types one letter at a time into a real `claude` in a throwaway named herdr session and prints the
composer rows, unstripped, from a few keys short of the edge to a few past the wrap, beside herdr's
own `pane.read` of the same rows. Run at more than one width to tell a fixed margin from a
proportional one.

`observe --cols` only clips the view; the pane keeps herdr's default width. The pane is sized by
holding `terminal session control` at the width under test, which is safe here only because the
session is this script's own.

Usage: composer-wrap.py [COLS ...]
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from importlib import import_module

composer = import_module("composer-line")

for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]


def raw(watch):
    with watch.lock:
        return ["".join(r) for r in watch.screen.g], watch.screen.x, watch.screen.y


def show(watch, label):
    rows, x, y = raw(watch)
    print(f"--- {label}: caret col {x} row {y}")
    for i in range(max(0, y - 3), min(len(rows), y + 3)):
        line = rows[i]
        edge = line[-8:]
        print(f"  {i:2d} |{line}|  last-nonblank {len(line.rstrip()) - 1}  edge {edge!r}")


def run(cols):
    composer.NAME = f"kampr-probe-wrap-{os.getpid()}-{cols}"
    home = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
    composer.SOCK = os.path.join(home, "herdr", "sessions", composer.NAME, "herdr.sock")
    composer.COLS = cols
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-wrap-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = composer.call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        env = dict(os.environ, HERDR_SOCKET_PATH=composer.SOCK)
        control = subprocess.Popen(
            ["herdr", "terminal", "session", "control", pane, "--cols", str(cols), "--rows", str(composer.ROWS)],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
        time.sleep(0.5)
        watch = composer.Watch(pane)
        time.sleep(1.0)
        composer.send(pane, "claude --model haiku\r")
        for _ in range(80):
            time.sleep(0.5)
            rows = raw(watch)[0]
            if any("trust" in r.lower() for r in rows):
                composer.send(pane, "\r")
                time.sleep(4)
                break
            if any(r.startswith("❯") for r in rows):
                break
        time.sleep(3)
        print(f"===== pane observed at {cols} cols")
        show(watch, "empty composer")
        typed = ""
        for i in range(cols + 8):
            ch = "abcdefghijklmnopqrstuvwxyz"[i % 26]
            composer.send(pane, ch)
            typed += ch
            time.sleep(0.35)
            if len(typed) >= cols - 5:
                show(watch, f"{len(typed)} typed")
                read = composer.call("pane.read", {"pane_id": pane, "source": "visible",
                                                   "format": "text", "strip_ansi": True})["read"]["text"]
                tail = read.rstrip("\n").split("\n")[-5:]
                for row in tail:
                    print(f"     herdr |{row}|")
        composer.send(pane, "\x7f" * len(typed))
        time.sleep(0.5)
        for _ in range(3):
            composer.send(pane, "\x03")
            time.sleep(0.3)
        watch.close()
        control.kill()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


def main():
    for cols in [int(a) for a in sys.argv[1:]] or [60, 95]:
        run(cols)


if __name__ == "__main__":
    main()
