#!/usr/bin/env python3
"""What Claude's tty looks like while it starts: when `icanon` and `icrnl` go off, measured against
when its composer is first drawn.

A reply sent before the box is drawn does not submit (`submit-while-booting.py`). This asks why:
the line discipline's cooked mode turns a carriage return into a newline and holds the line, so a
harness that has not yet put its tty in raw mode is handed the words with no submit on them. The
tty is read with `stty -F` off the harness's own fd 0, every 50 ms from the launch, in a throwaway
named herdr session (#97).
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-tty-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
RULE = "─" * 20


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def claude_pid(pane):
    fg = call("pane.process_info", {"pane_id": pane})["process_info"]["foreground_processes"]
    return next((p["pid"] for p in fg if p.get("name") == "claude"), None)


def flags(tty):
    out = subprocess.run(["stty", "-F", tty, "-a"], capture_output=True, text=True).stdout.split()
    return {f: (f in out) for f in ("icanon", "icrnl", "echo")}


def box_drawn(rows):
    return any(r.startswith("❯") for r in rows) and sum(1 for r in rows if r.startswith(RULE)) >= 2


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-tty-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        for n in range(4):
            ws = call("workspace.create", {"label": f"t{n}", "cwd": cwd, "focus": False})
            pane = ws["root_pane"]["pane_id"]
            watch = composer.Watch(pane)
            time.sleep(1.5)
            tty = os.readlink(f"/proc/{call('pane.process_info', {'pane_id': pane})['process_info']['foreground_processes'][0]['pid']}/fd/0")
            print(f"run {n}: shell at its prompt  {flags(tty)}")
            t0 = time.monotonic()
            call("pane.send_text", {"pane_id": pane, "text": "claude --model haiku\r"})
            last = None
            drawn = None
            while time.monotonic() - t0 < 15:
                t = time.monotonic() - t0
                f = flags(tty)
                key = tuple(sorted(f.items()))
                if key != last:
                    print(f"  {t:5.2f}s  {f}  claude pid={claude_pid(pane)}")
                    last = key
                rows, _, _ = watch.look()
                if drawn is None and box_drawn(rows):
                    drawn = t
                    print(f"  {t:5.2f}s  composer drawn")
                if drawn is not None and t > drawn + 1:
                    break
                time.sleep(0.05)
            if any("trust" in r.lower() for r in watch.look()[0]):
                call("pane.send_text", {"pane_id": pane, "text": "1"})
                time.sleep(3)
            for _ in range(3):
                call("pane.send_text", {"pane_id": pane, "text": "\x03"})
                time.sleep(0.3)
            watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
