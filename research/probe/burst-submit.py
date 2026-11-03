#!/usr/bin/env python3
"""Does a carriage return that follows a retyped line within milliseconds submit Claude, when the
line carries a pasted image's path? The conversation composer sends a reply holding a file as three
writes: the pane's line taken back with DELs, the path and the words, then `\r` (audit log,
2026-11-02T23:14:49.614, 316/404/1 bytes within 2 ms).

A booted, trusted `claude` in a throwaway named herdr session. Each trial types the words first, as
the mirror would have, then sends the three writes back to back (or with `gap` seconds before the
`\r`), and reads the screen: a submit is the marker in a user record of the transcript.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-burst-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
RULE = "─" * 20
IMAGE = sys.argv[1]


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def submitted(cwd, marker):
    import glob
    project = os.path.expanduser("~/.claude/projects/" + cwd.replace("/", "-"))
    for path in glob.glob(project + "/*.jsonl"):
        for line in open(path, errors="replace"):
            if '"type":"user"' in line.replace(" ", "") and marker in line:
                return True
    return False


def drawn(pane, tail, cap=8.0):
    start = time.time()
    while time.time() - start < cap:
        text = call("pane.read", {"pane_id": pane, "source": "visible", "format": "text", "strip_ansi": True})["read"]["text"]
        if tail in text:
            break
        time.sleep(0.03)
    return time.time() - start


def quiet(watch, still=0.25, cap=5.0):
    start = time.time()
    last, since = None, time.time()
    while time.time() - start < cap:
        now = watch.look()[0]
        if now != last:
            last, since = now, time.time()
        elif time.time() - since >= still:
            break
        time.sleep(0.03)
    return time.time() - start


def box(rows):
    rules = [i for i, r in enumerate(rows) if r.startswith(RULE)]
    return (rules[-2], rules[-1]) if len(rules) >= 2 else (None, None)


TRIALS = [
    ("words + bracketed picture, no gap", "bracketed", 0.0, False),
    ("words + bracketed picture, 300ms", "bracketed", 0.3, False),
]


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-burst-XXXXXX"], capture_output=True, text=True).stdout.strip()
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
        for n, (label, path, gap, dels) in enumerate(TRIALS[int(os.environ.get("FROM", "0")):] * int(os.environ.get("TIMES", "2"))):
            marker = f"BURST{n:02d}"
            words = f"{marker} reply with only the word ok"
            if path in ("report", "append"):
                words = f"{marker} reply with only the word ok, " + "ignoring this filler sentence here " * 8
            if path == "double":
                words = f"{marker} reply with only the word ok, " + "ignoring this filler sentence here " * 20
            if dels:
                for ch in words:
                    send(pane, ch)
                    time.sleep(0.02)
                time.sleep(1.5)
                if dels in (True, "split"):
                    send(pane, "\x7f" * len(words))
                elif dels == "clear":
                    send(pane, "\x03")
                if dels == "split":
                    time.sleep(1.0)
            send(pane, {"front": f"{IMAGE} {words}", "back": f"{words} {IMAGE}", "slash": f"/tmp {words}", "long": f"{words} " + "and some more words " * 5, "report": f"{IMAGE} {words} " + "and then some more of what I wanted to say " * 6 + f"END{marker}", "bracketed": f"{words} \x1b[200~{IMAGE}\x1b[201~", "double": f"{IMAGE} {words} " + "and then some more of what I wanted to say " * 15, "append": f" {IMAGE}", None: words}[path])
            if gap in ("drawn", "drawn+"):
                waited = drawn(pane, f"END{marker}")
                print(f"    drawn after {waited:.2f}s", flush=True)
                if gap == "drawn+":
                    time.sleep(0.3)
            elif gap == "quiet":
                waited = quiet(watch)
                print(f"    quiet after {waited:.2f}s", flush=True)
            elif gap:
                time.sleep(gap)
            send(pane, "\r")
            for t in range(int(os.environ.get("WATCH", "4"))):
                time.sleep(1)
                rows, x, y = watch.look()
                top, bottom = box(rows)
                if os.environ.get("TRACE"):
                    print(f"    t={t+1}s submitted={submitted(cwd, marker)} box rows={bottom - top - 1 if top is not None else None} caret=({x},{y}) last={rows[bottom-1][-40:] if top is not None else None!r}", flush=True)
            rows, x, y = watch.look()
            top, bottom = box(rows)
            above = submitted(cwd, marker)
            inside = [r for r in rows[top + 1:bottom]] if top is not None else []
            print(f"  {label:28s}: {'SUBMIT' if above else 'HELD'}  box rows={len(inside)}", flush=True)
            if os.environ.get("DUMP"):
                for r in rows:
                    if r.strip(): print(f"    # {r[:110]!r}", flush=True)
            for r in inside:
                print(f"      | {r[:100]!r}", flush=True)
            if above:
                time.sleep(8)
            else:
                send(pane, "\x03")
                time.sleep(1)
                pass
                time.sleep(1)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
