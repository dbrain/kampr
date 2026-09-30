#!/usr/bin/env python3
"""Does typing into Claude get slower as its conversation grows — with no Kampr on the path?

Keypress-to-glyph is timed from `pane.send_text` to the first `observe` frame whose screen shows
the character, in a throwaway named herdr session with a real `claude`. Measured on a fresh session
and again after each of several long answers, so a latency that grows with the transcript is
Claude's own render and not anything a client or node does.

Usage: claude-echo-growth.py [ROUNDS] [KEYS]
"""
import os, shutil, statistics, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-echo-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
composer.COLS = 200
for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]

LONG = ("Write about {n} different kinds of trees, one paragraph of four sentences each, plain prose, "
        "no tools, no headings.")


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def composer_row(watch):
    rows = watch.look()[0]
    for r in reversed(rows):
        if r.startswith("❯"):
            return r
    return ""


def echo(pane, watch, keys):
    """Type `keys` distinct letters one at a time, timing each to its glyph; then erase them."""
    out = []
    typed = ""
    for i in range(keys):
        ch = "abcdefghijklmnopqrstuvwxyz"[i % 26]
        typed += ch
        t0 = time.perf_counter()
        send(pane, ch)
        while time.perf_counter() - t0 < 5:
            if composer_row(watch).rstrip().endswith(typed):
                break
            time.sleep(0.0005)
        out.append((time.perf_counter() - t0) * 1000)
        time.sleep(0.12)
    send(pane, "\x7f" * len(typed))
    time.sleep(0.5)
    return out


def idle(watch, seconds=180):
    end = time.time() + seconds
    time.sleep(3)
    while time.time() < end:
        rows = watch.look()[0]
        if not any("esc to interrupt" in r.lower() for r in rows):
            return
        time.sleep(0.5)


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    keys = int(sys.argv[2]) if len(sys.argv) > 2 else 20
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-echo-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        for _ in range(60):
            time.sleep(0.5)
            rows = watch.look()[0]
            if any("trust" in r.lower() for r in rows):
                send(pane, "\r")
                time.sleep(4)
                break
            if any(r.startswith("❯") for r in rows):
                break
        time.sleep(2)
        for n in range(rounds + 1):
            ms = sorted(echo(pane, watch, keys))
            print(f"after {n} long answers: p50 {statistics.median(ms):6.1f} ms   "
                  f"p90 {ms[int(len(ms) * 0.9) - 1]:6.1f} ms   max {ms[-1]:6.1f} ms", flush=True)
            if n == rounds:
                break
            send(pane, LONG.format(n=12))
            time.sleep(0.4)
            send(pane, "\r")
            idle(watch)
            time.sleep(2)
        for _ in range(3):
            send(pane, "\x03")
            time.sleep(0.3)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
