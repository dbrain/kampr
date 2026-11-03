#!/usr/bin/env python3
"""Does Claude take a bracketed paste of an image path, mid-line, as `[Image #n]`, and what does
its record hold when the line is sent? A booted, trusted `claude` in a throwaway named herdr
session: words typed a key at a time, the caret moved left into them, the path pasted bracketed,
the box read, then Enter and the transcript's user record read.
"""
import glob, json, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")
HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-bimg-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
IMAGE = sys.argv[1]


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def show(watch, label):
    rows, x, y = watch.look()
    print(f"-- {label} caret=({x},{y})")
    for r in rows:
        if r.strip():
            print(f"   {r[:110]!r}")


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-bimg-XXXXXX"], capture_output=True, text=True).stdout.strip()
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
        for ch in "BIMG what colour is this picture":
            send(pane, ch)
            time.sleep(0.02)
        time.sleep(1)
        send(pane, "\x1b[D" * 13)
        time.sleep(0.5)
        send(pane, f"\x1b[200~{IMAGE}\x1b[201~")
        time.sleep(2)
        show(watch, "after bracketed paste mid-line")
        send(pane, "\r")
        time.sleep(10)
        show(watch, "after Enter")
        project = os.path.expanduser("~/.claude/projects/" + cwd.replace("/", "-"))
        for path in glob.glob(project + "/*.jsonl"):
            for line in open(path):
                r = json.loads(line)
                if r.get("type") == "user" and not r.get("isMeta"):
                    c = r["message"]["content"]
                    print("RECORD", repr(c)[:300] if isinstance(c, str) else [(b.get("type"), (b.get("text") or "")[:200]) for b in c])
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
