#!/usr/bin/env python3
"""Whether a reply sent the way the conversation composer sends it — the text as one
`pane.send_text`, then `\r` as a second — submits Claude's prompt box or only opens a line in it.

Sweeps the length of the text and the gap between the two writes, against a freshly booted Claude
in a throwaway named herdr session of its own (#97), torn down at the end. A submit is read two
ways: `agent_status` going `working`, and the text leaving the composer. Prints what it measured.
"""
import json, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-submit-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def status(pane):
    return call("pane.get", {"pane_id": pane})["pane"].get("agent_status")


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def wait_idle(pane, limit=60):
    end = time.time() + limit
    while time.time() < end:
        if status(pane) in ("idle", "done"):
            return True
        time.sleep(0.5)
    return False


def trial(pane, watch, n, length, gap):
    marker = f"PROBE{n:03d}"
    head = f"{marker} reply with only the word ok. "
    text = (head + "filler " * length)[: max(length, len(head))]
    t0 = time.monotonic()
    send(pane, text)
    t1 = time.monotonic()
    if gap:
        time.sleep(gap)
    send(pane, "\r")
    t2 = time.monotonic()
    worked = False
    end = time.time() + 4
    while time.time() < end:
        if status(pane) == "working":
            worked = True
            break
        time.sleep(0.2)
    time.sleep(0.8)
    rows, x, y = watch.look()
    caret = rows[y]
    in_box = marker in caret or any(marker in r for r in rows[max(0, y - 30): y + 1] if r.lstrip().startswith("│") or r.startswith("> "))
    pasted = any("Pasted text" in r for r in rows)
    verdict = "SUBMIT" if worked else "HELD"
    print(f"  len={len(text):5d} gap={gap*1000:6.1f}ms  text-rpc={1000*(t1-t0):5.1f}ms cr-after={1000*(t2-t1):5.1f}ms  "
          f"{verdict:6s} pasted-chip={pasted}  caret row {caret[:70]!r}")
    if worked:
        wait_idle(pane)
    else:
        send(pane, "\x03")
        time.sleep(1.0)
    return worked


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-submit-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        time.sleep(12)
        rows, x, y = watch.look()
        if any("trust" in r.lower() for r in rows):
            send(pane, "1")
            time.sleep(6)
        print("booted; caret row:", repr(watch.look()[0][watch.look()[2]]))
        n = 0
        plan = [(l, 0.0) for l in (40, 400, 900, 1500, 3000)] * 2
        plan += [(3000, g) for g in (0.05, 0.15, 0.3, 0.6)]
        for length, gap in plan:
            n += 1
            trial(pane, watch, n, length, gap)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
