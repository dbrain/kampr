#!/usr/bin/env python3
"""Whether a reply sent to a Claude that is still starting submits, and what herdr says about the
pane while it starts.

Each trial launches a fresh `claude` in its own workspace of a throwaway named herdr session (#97),
sends a reply the way the conversation composer does — the text, then `\r` as a second
`pane.send_text` — at a fixed delay after the launch, and reads whether it submitted. `agent_status`
and whether the prompt box is drawn are sampled every 100 ms from the launch, so the moment the
harness is ready can be compared with the moment the reply landed. The directory is trusted once up
front so the trust dialog is not what is measured.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-boot-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
CLAUDE = "claude --model haiku"


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def pane_state(pane):
    p = call("pane.get", {"pane_id": pane})["pane"]
    return p.get("agent"), p.get("agent_status")


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def prompt_drawn(rows):
    return any(r.startswith("❯") for r in rows)


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
    send(pane, "\x03")
    time.sleep(0.5)
    send(pane, "\x03")
    time.sleep(1.5)
    watch.close()


def trial(cwd, n, delay):
    pane, watch = fresh(cwd, f"t{n}")
    marker = f"BOOT{n:03d}"
    t0 = time.monotonic()
    send(pane, CLAUDE + "\r")
    first_agent = first_status = first_prompt = None
    sent_at = None
    while True:
        t = time.monotonic() - t0
        agent, status = pane_state(pane)
        rows, _, _ = watch.look()
        if agent and first_agent is None:
            first_agent = (t, agent)
        if status and first_status is None:
            first_status = (t, status)
        if first_prompt is None and prompt_drawn(rows):
            first_prompt = t
        if sent_at is None and t >= delay:
            send(pane, f"{marker} reply with only the word ok")
            send(pane, "\r")
            sent_at = t
        if sent_at is not None and t > max(delay + 6, 9):
            break
        time.sleep(0.1)
    worked = False
    for _ in range(40):
        if pane_state(pane)[1] == "working":
            worked = True
            break
        time.sleep(0.2)
    rows, x, y = watch.look()
    lines = [r for r in rows if r.strip()]
    box = lines[-12:]
    fmt = lambda v: "-" if v is None else (f"{v[0]:.1f}s {v[1]}" if isinstance(v, tuple) else f"{v:.1f}s")
    print(f"  delay={delay:4.1f}s  sent@{sent_at:4.1f}s  agent={fmt(first_agent):18s} status={fmt(first_status):16s} "
          f"prompt={fmt(first_prompt):6s}  {'SUBMIT' if worked else 'HELD'}")
    if not worked:
        for r in box:
            print(f"      | {r[:100]!r}")
    watch.close()
    send(pane, "\x03")
    time.sleep(0.3)
    send(pane, "\x03")
    time.sleep(0.3)
    send(pane, "\x03")


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-boot-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        trust(cwd)
        n = 0
        for delay in (0.3, 0.8, 1.5, 2.5, 4.0, 7.0) * 2:
            n += 1
            trial(cwd, n, delay)
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
