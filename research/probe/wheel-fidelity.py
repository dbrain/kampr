#!/usr/bin/env python3
"""Does Kampr's emulator still agree with herdr after a Claude pane is scrolled by the wheel?

A phone scrolls a Claude pane by sending SGR wheel reports at `PaneScroll`'s 40 ms pace (#528),
and Claude answers each by redrawing its transcript. This fills a real `claude` in a throwaway
herdr session with numbered lines, then runs the fidelity canary (`kampr-spike`) — which rebuilds
the grid from the `observe` frames alone — while the reports go in, and compares that grid against
herdr's own `pane.read visible` at the end. A row that differs is a row a phone draws wrong.

Usage: wheel-fidelity.py [REPORTS] [PACE_MS] [DIRECTION up|down|both]
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-wheel-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]
SPIKE = os.path.join(HERE, "../../target/debug/kampr-spike")


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def visible(pane):
    return call("pane.read", {"pane_id": pane, "source": "visible", "lines": 80, "format": "text"})["read"]["text"]


def main():
    reports = int(sys.argv[1]) if len(sys.argv) > 1 else 30
    pace = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    direction = sys.argv[3] if len(sys.argv) > 3 else "both"
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-wheel-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        for _ in range(60):
            time.sleep(0.5)
            text = visible(pane)
            if "trust" in text.lower():
                send(pane, "\r")
                time.sleep(4)
                break
            if "\n❯" in text:
                break
        send(pane, "Print the numbers 1 to 150, one per line, each followed by a dash and its English "
                   "word. No tools, nothing else.")
        time.sleep(0.5)
        send(pane, "\r")
        for _ in range(120):
            time.sleep(1)
            if "150" in visible(pane) and "esc to interrupt" not in visible(pane).lower():
                break
        time.sleep(2)

        env = dict(os.environ, HERDR_SOCKET_PATH=SOCK, KAMPR_SPIKE_SECS=str(3 + reports * pace // 1000 * 2 + 4))
        spike = subprocess.Popen([SPIKE, pane], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        time.sleep(1.5)
        plan = {"up": [64] * reports, "down": [65] * reports, "both": [64] * reports + [65] * (reports // 2)}[direction]
        for button in plan:
            send(pane, f"\x1b[<{button};40;20M")
            time.sleep(pace / 1000)
        out, _ = spike.communicate(timeout=120)
        print(out)
        print("herdr's own visible screen:")
        print(visible(pane))
        for _ in range(3):
            send(pane, "\x03")
            time.sleep(0.3)
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
