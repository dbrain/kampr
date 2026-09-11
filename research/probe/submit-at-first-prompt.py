#!/usr/bin/env python3
"""How soon after Claude first draws its prompt box a reply sent the composer's way submits.

`submit-while-booting.py` found every reply sent before the box was drawn held and every one sent
after it submitted, at a 100 ms sampling grain. This narrows the grain: the screen is polled every
20 ms from the launch, and the reply goes out a fixed settle after the first frame that shows the
box. A submit is read off the screen — the marker on a `❯` row *above* the box's top rule — because
`agent_status` does not move in a session started from inside another Claude.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-first-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
CLAUDE = "claude --model haiku"
RULE = "─" * 20


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def box_drawn(rows):
    return any(r.startswith("❯") for r in rows) and sum(1 for r in rows if r.startswith(RULE)) >= 2


def submitted(rows, marker):
    rules = [i for i, r in enumerate(rows) if r.startswith(RULE)]
    if len(rules) < 2:
        return False
    top = rules[-2]
    return any(marker in r for r in rows[:top])


def fresh(cwd, label):
    ws = call("workspace.create", {"label": label, "cwd": cwd, "focus": False})
    pane = ws["root_pane"]["pane_id"]
    watch = composer.Watch(pane)
    time.sleep(1.5)
    return pane, watch


def trust(cwd):
    pane, watch = fresh(cwd, "trust")
    send(pane, CLAUDE + "\r")
    time.sleep(12)
    rows, _, _ = watch.look()
    if any("trust" in r.lower() for r in rows):
        send(pane, "1")
        time.sleep(5)
    for _ in range(2):
        send(pane, "\x03")
        time.sleep(0.5)
    watch.close()


def trial(cwd, n, settle):
    pane, watch = fresh(cwd, f"t{n}")
    marker = f"FIRST{n:03d}"
    t0 = time.monotonic()
    send(pane, CLAUDE + "\r")
    drawn = None
    while time.monotonic() - t0 < 20:
        rows, _, _ = watch.look()
        if box_drawn(rows):
            drawn = time.monotonic() - t0
            break
        time.sleep(0.02)
    if drawn is None:
        print(f"  settle={settle*1000:5.0f}ms  box never drawn")
        watch.close()
        return
    time.sleep(settle)
    send(pane, f"{marker} reply with only the word ok")
    send(pane, "\r")
    ok = False
    for _ in range(40):
        rows, _, _ = watch.look()
        if submitted(rows, marker):
            ok = True
            break
        time.sleep(0.2)
    print(f"  settle={settle*1000:5.0f}ms  box@{drawn:4.2f}s  {'SUBMIT' if ok else 'HELD'}")
    if not ok:
        for r in [r for r in rows if r.strip()][-8:]:
            print(f"      | {r[:100]!r}")
    for _ in range(3):
        send(pane, "\x03")
        time.sleep(0.3)
    watch.close()


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-first-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        trust(cwd)
        n = 0
        for settle in (0.0, 0.0, 0.0, 0.0, 0.05, 0.05, 0.05, 0.15, 0.15, 0.15, 0.4, 0.4):
            n += 1
            trial(cwd, n, settle)
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
