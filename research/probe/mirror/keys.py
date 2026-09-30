#!/usr/bin/env python3
"""The editing keys a two-way mirror of a harness's composer would press, measured per harness:
what one backspace takes, where typed text lands after left-arrows, how the box wraps words and
what the wrapped rows read back as, where the caret sits in a wrapped line, which key writes a
newline without submitting, where a single `pane.send_text` burst turns into a paste placeholder,
and what ctrl+c does to an empty box.

Throwaway named herdr session per harness, torn down; the caret is read off a real `observe`
stream folded through vt.py, the grid Kampr's own emulator builds.

Usage: keys.py [claude|codex|agy ...]
"""
import os, shutil, sys, time
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from importlib import import_module
import json, subprocess
from common import CLEAN_ENV

os.environ.clear()
os.environ.update(CLEAN_ENV)
cl = import_module("composer-line")
cl.COLS, cl.ROWS = 80, 40

MARKERS = {"claude": "❯", "codex": "›", "agy": ">"}
BOOT = {
    "claude": ("claude --model haiku", 12, []),
    "codex": ("codex", 12, [("\r", 6)]),
    "agy": ("agy", 16, [("\r", 10)]),
}


LOOK = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(HERE))), "target/debug/examples/composer_look")


class Look:
    def __init__(self, pane, agent):
        self.child = subprocess.Popen([LOOK, cl.SOCK, pane, str(cl.COLS), str(cl.ROWS), agent],
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                                      env=dict(os.environ, HERDR_SOCKET_PATH=cl.SOCK))

    def look(self):
        self.child.stdin.write("look\n"); self.child.stdin.flush()
        return json.loads(self.child.stdout.readline())

    def close(self):
        self.child.kill()


def box(watch, marker):
    got = watch.look()
    rows, (x, y) = got["rows"], got["caret"]
    head = max((i for i, r in enumerate(rows) if r.startswith(marker)), default=None)
    if head is None:
        return None, x, y, rows, got["read"]
    last = head
    while last + 1 < len(rows) and rows[last + 1].startswith("  ") and rows[last + 1].strip():
        last += 1
    return rows[head:last + 1], x, y - head, rows, got["read"]


def show(label, watch, marker):
    rows, x, dy, _, read = box(watch, marker)
    print(f"  {label}: caret col {x} box-row {dy}  reader={read!r}")
    for r in rows or []:
        print(f"     |{r.rstrip()}|  (trailing blanks {len(r) - len(r.rstrip())})")
    return rows, x, dy


def settle(t=0.9):
    time.sleep(t)


def erase(pane, n=400):
    cl.send(pane, "\x05")
    settle(0.3)
    cl.send(pane, "\x7f" * n)
    settle(1.2)


def run(name):
    cmd, boot, extra = BOOT[name]
    marker = MARKERS[name]
    ws = cl.call("workspace.create", {"label": name, "cwd": CWD, "focus": False})
    pane = ws["root_pane"]["pane_id"]
    print(f"\n{'=' * 80}\n{name}\n{'=' * 80}")
    control = subprocess.Popen(
        ["herdr", "terminal", "session", "control", pane, "--cols", str(cl.COLS), "--rows", str(cl.ROWS)],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env=dict(os.environ, HERDR_SOCKET_PATH=cl.SOCK))
    settle(0.5)
    watch = Look(pane, name)
    settle(1)
    cl.send(pane, cmd + "\r")
    time.sleep(boot)
    for keys, pause in extra:
        cl.send(pane, keys)
        time.sleep(pause)
    if name == "claude":
        rows = watch.look()["rows"]
        if any("trust this folder" in r for r in rows):
            cl.send(pane, "\x1b[B"); settle(0.4); cl.send(pane, "\r"); time.sleep(4)
    show("empty", watch, marker)

    print("-- backspace")
    for ch in "alpha beta gamma":
        cl.send(pane, ch); time.sleep(0.05)
    settle()
    cl.send(pane, "\x7f"); settle()
    show("after one \\x7f", watch, marker)
    cl.send(pane, "\x7f" * 3); settle()
    show("after three more", watch, marker)

    print("-- left arrows then insert")
    cl.send(pane, "\x1b[D" * 4); settle()
    show("after 4 left", watch, marker)
    cl.send(pane, "X"); settle()
    show("after typing X", watch, marker)
    cl.send(pane, "\x1b[C" * 4); settle()
    show("after 4 right", watch, marker)
    cl.send(pane, "Z"); settle()
    show("after typing Z", watch, marker)
    erase(pane)

    print("-- wrapping words (typed in 8-char chunks)")
    words = ("the quick brown fox jumps over the lazy dog and then keeps running past the "
             "old mill until the river bends away toward the far hills end")
    for i in range(0, len(words), 8):
        cl.send(pane, words[i:i + 8]); time.sleep(0.12)
    settle(1.5)
    rows, x, dy = show("wrapped", watch, marker)
    print(f"     typed {len(words)} chars")
    cl.send(pane, "\x1b[D" * 30); settle()
    show("after 30 left", watch, marker)
    cl.send(pane, "Y"); settle()
    show("after typing Y", watch, marker)
    cl.send(pane, "\x05"); settle()
    cl.send(pane, "\x1b[D" * 70); settle()
    show("after ctrl+e then 70 left (crosses the wrap)", watch, marker)
    cl.send(pane, "W"); settle()
    show("after typing W", watch, marker)
    cl.send(pane, "\x1b[C" * 70); settle()
    show("after 70 right", watch, marker)
    cl.send(pane, "\x01"); settle()
    show("after ctrl+a", watch, marker)
    cl.send(pane, "\x1b[H"); settle()
    show("after Home", watch, marker)
    cl.send(pane, "\x05"); settle()
    show("after ctrl+e", watch, marker)
    erase(pane)

    print("-- newline candidates")
    for label, key in [("meta-enter \\x1b\\r", "\x1b\r"), ("ctrl+j \\n", "\n"),
                       ("backslash-enter \\\\\\r", "\\\r"), ("kitty shift-enter", "\x1b[13;2u")]:
        cl.send(pane, "one"); settle(0.5)
        cl.send(pane, key); settle(0.8)
        cl.send(pane, "two"); settle(1.0)
        rows, x, dy, allrows, _ = box(watch, marker)
        submitted = any(r.strip().endswith("one") for r in allrows[:-6]) and not rows
        show(f"{label}", watch, marker)
        erase(pane)
        time.sleep(1.5 if submitted else 0)

    print("-- paste bursts")
    for n in (40, 300, 800, 1500, 3000):
        cl.send(pane, "".join("abcdefghij"[i % 10] for i in range(n))); settle(1.5)
        rows, x, dy, _, read = box(watch, marker)
        print(f"  burst {n}: first row {(rows or [''])[0].rstrip()[:90]!r} rows {len(rows or [])}")
        erase(pane, n + 20)
    cl.send(pane, "line one\nline two"); settle(1.5)
    show("burst with a newline in it", watch, marker)
    erase(pane)
    cl.send(pane, "\n".join(f"row {i}" for i in range(12))); settle(1.5)
    show("burst of twelve lines", watch, marker)
    erase(pane)

    print("-- ctrl+c on an empty box")
    settle(1)
    cl.send(pane, "\x03"); settle(1.2)
    got = watch.look(); rows, (x, y) = got["rows"], got["caret"]
    for r in rows[max(0, y - 2):y + 4]:
        print(f"     |{r}|")
    settle(3)
    watch.close()
    control.kill()


def main():
    global CWD
    CWD = __import__("subprocess").run(["mktemp", "-d", "/tmp/kampr-keys-XXXXXX"], capture_output=True, text=True).stdout.strip()
    cl.start()
    try:
        for name in sys.argv[1:] or ["claude", "codex", "agy"]:
            try:
                run(name)
            except Exception as e:
                import traceback; traceback.print_exc()
    finally:
        cl.stop()
        shutil.rmtree(CWD, ignore_errors=True)


if __name__ == "__main__":
    main()
