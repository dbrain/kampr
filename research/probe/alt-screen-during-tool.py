#!/usr/bin/env python3
"""Does herdr's ring come back while Claude is running a tool?

The node drops the shell era the moment a harness owns the screen (`ScrollbackRing::superseded`,
#244), and it can only refill from a `pane.read` that answers. So if Claude leaves the alternate
screen for the length of a tool call — a Bash command, a subprocess taking the terminal — herdr
hands the shell era back mid-session and the node ingests it again, which puts a pre-harness ring
under a live conversation and gives the wheel back to Kampr.

`scroll.max_offset_from_bottom` is sampled every 200 ms through a real turn in a throwaway named
herdr session (#97), and every non-zero reading is printed with what the screen said at the time.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-alt-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def depth(pane):
    scroll = call("pane.get", {"pane_id": pane})["pane"].get("scroll") or {}
    return scroll.get("max_offset_from_bottom", 0)


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def sample(pane, watch, seconds, label):
    end = time.time() + seconds
    seen = []
    while time.time() < end:
        d = depth(pane)
        if d and (not seen or seen[-1][1] != d):
            rows, _, y = watch.look()
            seen.append((round(time.time() % 1000, 1), d))
            print(f"  {label}: ring back, max_offset={d}  caret row {rows[y][:70]!r}")
        time.sleep(0.2)
    if not seen:
        print(f"  {label}: ring stayed gone for {seconds}s")
    return seen


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-alt-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        # A shell era worth noticing, so a refill is unmistakable.
        send(pane, "seq 1 200\r")
        time.sleep(2)
        print(f"shell era before the harness: max_offset={depth(pane)}")
        send(pane, "claude --model haiku\r")
        time.sleep(12)
        if any("trust" in r.lower() for r in watch.look()[0]):
            send(pane, "1")
            time.sleep(6)
        print(f"claude up: max_offset={depth(pane)}")
        sample(pane, watch, 4, "idle")

        # A turn that certainly runs a tool: the harness spawns a child that wants the terminal.
        send(pane, "run the bash command `seq 1 50` and then say done")
        send(pane, "\r")
        sample(pane, watch, 45, "during a tool call")

        print(f"after the turn: max_offset={depth(pane)}")
        for _ in range(3):
            send(pane, "\x03")
            time.sleep(0.4)
        time.sleep(1.5)
        print(f"after quitting claude: max_offset={depth(pane)}")
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
