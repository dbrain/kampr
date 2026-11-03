#!/usr/bin/env python3
"""What each harness makes of an image path pasted bracketed into the middle of its box, and how
its box edits around what it draws for it: what one backspace after it takes, where a left arrow
from just after it lands, what Kampr's own composer reader reads back, and what the sent message
holds.

The same throwaway session, emulator and reader as keys.py.

Usage: image-token.py <image> [claude|codex|agy ...]
"""
import glob, json, os, shutil, subprocess, sys, time
from importlib import import_module

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
keys = import_module("keys")
cl = keys.cl
IMAGE = sys.argv[1]
# Codex now opens on an update prompt whose first choice, and Enter, is to install; Esc skips it.
keys.BOOT["codex"] = ("codex", 10, [("\x1b", 3)])


def paste(pane):
    cl.send(pane, f"\x1b[200~{IMAGE}\x1b[201~")
    keys.settle(2.0)


def run(name):
    cmd, boot, extra = keys.BOOT[name]
    marker = keys.MARKERS[name]
    ws = cl.call("workspace.create", {"label": name, "cwd": keys.CWD, "focus": False})
    pane = ws["root_pane"]["pane_id"]
    print(f"\n{'=' * 80}\n{name}\n{'=' * 80}", flush=True)
    control = subprocess.Popen(
        ["herdr", "terminal", "session", "control", pane, "--cols", str(cl.COLS), "--rows", str(cl.ROWS)],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env=dict(os.environ, HERDR_SOCKET_PATH=cl.SOCK))
    keys.settle(0.5)
    watch = keys.Look(pane, name)
    keys.settle(1)
    cl.send(pane, cmd + "\r")
    time.sleep(boot)
    for k, pause in extra:
        cl.send(pane, k)
        time.sleep(pause)
    if name == "claude":
        if any("trust this folder" in r for r in watch.look()["rows"]):
            cl.send(pane, "\x1b[B"); keys.settle(0.4); cl.send(pane, "\r"); time.sleep(4)
    keys.show("empty", watch, marker)

    def fresh(label):
        for ch in "alpha beta gamma":
            cl.send(pane, ch); time.sleep(0.05)
        keys.settle()
        cl.send(pane, "\x1b[D" * 6); keys.settle()
        paste(pane)
        keys.show(label, watch, marker)

    print("-- pasted mid-line, between `beta ` and `gamma`", flush=True)
    fresh("after the paste")
    cl.send(pane, "\x7f"); keys.settle()
    keys.show("one \\x7f after it", watch, marker)
    cl.send(pane, "\x7f"); keys.settle()
    keys.show("a second \\x7f", watch, marker)
    keys.erase(pane)

    fresh("pasted for two lefts")
    cl.send(pane, "\x1b[D" * 2); keys.settle()
    cl.send(pane, "X"); keys.settle()
    keys.show("two left then X", watch, marker)
    keys.erase(pane)

    fresh("pasted again")
    cl.send(pane, "\x1b[D"); keys.settle()
    cl.send(pane, "X"); keys.settle()
    keys.show("one left then X", watch, marker)
    cl.send(pane, "\x1b[C" * 2); keys.settle()
    cl.send(pane, "Y"); keys.settle()
    keys.show("two right then Y", watch, marker)
    keys.erase(pane)

    print("-- sent", flush=True)
    for ch in "TOKEN what colour is this picture":
        cl.send(pane, ch); time.sleep(0.03)
    keys.settle()
    cl.send(pane, "\x1b[D" * 13); keys.settle()
    paste(pane)
    keys.show("before Enter", watch, marker)
    cl.send(pane, "\r")
    time.sleep(15)
    rows = watch.look()["rows"]
    for r in rows:
        if r.strip():
            print(f"     |{r.rstrip()[:100]}|")
    if name == "claude":
        project = os.path.expanduser("~/.claude/projects/" + keys.CWD.replace("/", "-"))
        for path in glob.glob(project + "/*.jsonl"):
            for line in open(path):
                r = json.loads(line)
                if r.get("type") == "user" and not r.get("isMeta"):
                    c = r["message"]["content"]
                    print("  RECORD", repr(c)[:200] if isinstance(c, str) else [(b.get("type"), (b.get("text") or "")[:120]) for b in c])
    if name == "codex":
        newest = sorted(glob.glob(os.path.expanduser("~/.codex/sessions/**/*.jsonl"), recursive=True), key=os.path.getmtime)[-1:]
        for path in newest:
            for line in open(path):
                if "TOKEN" in line and '"role":"user"' in line.replace(" ", ""):
                    r = json.loads(line)
                    p = r.get("payload", r)
                    print("  RECORD", [(b.get("type"), (b.get("text") or str(b.get("image_url", ""))[:60])[:120]) for b in p.get("content", [])])
    watch.close()
    control.kill()


def main():
    keys.CWD = subprocess.run(["mktemp", "-d", "/tmp/kampr-imgtok-XXXXXX"], capture_output=True, text=True).stdout.strip()
    cl.start()
    try:
        for name in sys.argv[2:] or ["claude", "codex", "agy"]:
            try:
                run(name)
            except Exception:
                import traceback; traceback.print_exc()
    finally:
        cl.stop()
        shutil.rmtree(keys.CWD, ignore_errors=True)


if __name__ == "__main__":
    main()
