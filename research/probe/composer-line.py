#!/usr/bin/env python3
"""What a harness's composer line looks like with the operator's text half-typed in it, where the
caret sits, and what clears it.

Runs in a throwaway named herdr session of its own, torn down at the end (#97: a node serves every
session it can find). The screen and the caret are read off a real `terminal session observe`
stream through research/probe/vt.py, which is the same grid Kampr's own emulator builds — so the
caret column measured here is the one `PaneRegistry` would report. Prints what it measured; leaves
nothing running.
"""
import base64, json, os, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import vt

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-composer-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
COLS, ROWS = 95, 40


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def start():
    subprocess.Popen(["herdr", "server", "--session", NAME],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(150):
        if os.path.exists(SOCK):
            time.sleep(0.6)
            return
        time.sleep(0.1)
    raise SystemExit("herdr never came up")


def stop():
    try:
        rpc("server.stop", {}, sock_path=SOCK)
    except Exception:
        pass
    for _ in range(60):
        if not os.path.exists(SOCK):
            break
        time.sleep(0.1)
    shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)


class Watch:
    """A live `observe` stream folded into a VT screen, so the caret is a measurement."""

    def __init__(self, pane):
        self.screen = vt.Screen(COLS, ROWS)
        self.lock = threading.Lock()
        env = dict(os.environ, HERDR_SOCKET_PATH=SOCK)
        self.child = subprocess.Popen(
            ["herdr", "terminal", "session", "observe", pane, "--cols", str(COLS), "--rows", str(ROWS)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, env=env)
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.child.stdout:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if rec.get("type") != "terminal.frame":
                continue
            data = base64.b64decode(rec["bytes"]).decode("utf-8", "replace")
            with self.lock:
                vt.feed(self.screen, data)

    def look(self):
        with self.lock:
            rows = ["".join(r).rstrip() for r in self.screen.g]
            return rows, self.screen.x, self.screen.y

    def close(self):
        self.child.kill()


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def report(watch, label):
    rows, x, y = watch.look()
    print(f"  {label}")
    print(f"    caret col={x} row={y}   caret row reads {rows[y]!r}")
    lo = max(0, y - 3)
    for i in range(lo, min(ROWS, y + 3)):
        print(f"      [{i}]{'*' if i == y else ' '}{rows[i]!r}")
    return rows, x, y


def trial(pane, watch, typed, keys, label):
    send(pane, typed)
    time.sleep(1.4)
    before, bx, _ = watch.look()
    hit_before = sum(1 for r in before if typed in r)
    send(pane, keys)
    time.sleep(1.4)
    after, ax, ay = watch.look()
    hit_after = sum(1 for r in after if typed in r)
    print(f"  {label}: lines carrying the typed text {hit_before} -> {hit_after}; "
          f"caret col {bx} -> {ax}")
    print(f"    caret row after: {after[ay]!r}")
    return hit_after == 0


def run(harness, cmd, boot, cwd, settle):
    ws = call("workspace.create", {"label": harness, "cwd": cwd})
    pane = ws["root_pane"]["pane_id"]
    print(f"\n{'=' * 90}\n{harness}  (pane {pane})\n{'=' * 90}")
    watch = Watch(pane)
    time.sleep(1.0)
    send(pane, cmd + "\r")
    time.sleep(boot)
    for keys, pause in settle:
        send(pane, keys)
        time.sleep(pause)
    report(watch, "EMPTY composer, freshly booted:")

    typed = "the quick brown fox"
    send(pane, typed)
    time.sleep(1.5)
    report(watch, f"after send_text({typed!r}):")

    send(pane, "\x15")
    time.sleep(1.2)
    report(watch, "after ctrl+u:")

    print("  --- what clears it")
    for label, keys in [("ctrl+u        (\\x15)", "\x15"),
                        ("ctrl+a ctrl+k (\\x01\\x0b)", "\x01\x0b"),
                        ("19 backspaces (\\x7f)", "\x7f" * 19)]:
        trial(pane, watch, typed, keys, label)
        send(pane, "\x15")
        time.sleep(0.8)

    print("  --- ctrl+u is undoable?")
    send(pane, typed)
    time.sleep(1.2)
    send(pane, "\x15")
    time.sleep(1.0)
    send(pane, "\x19")
    time.sleep(1.2)
    rows, x, y = watch.look()
    print(f"    after ctrl+u then ctrl+y: caret row {rows[y]!r}  (col {x})")
    send(pane, "\x15")
    time.sleep(0.8)

    print("  --- two send_texts with no submit (the concatenation being reported)")
    send(pane, "half a sentence ")
    time.sleep(1.0)
    send(pane, "and the rest")
    time.sleep(1.2)
    rows, x, y = watch.look()
    print(f"    caret row {rows[y]!r}")
    send(pane, "\x15")
    time.sleep(0.8)

    print("  --- a 200-character line, to see how a wrapped composer paints")
    send(pane, "x" * 200)
    time.sleep(1.6)
    rows, x, y = watch.look()
    for i in range(max(0, y - 5), min(ROWS, y + 3)):
        print(f"      [{i}]{'*' if i == y else ' '}{rows[i]!r}")
    send(pane, "\x15")
    time.sleep(0.8)

    print("  --- empty again, sampled six times to see whether the hint rotates")
    for _ in range(6):
        rows, x, y = watch.look()
        print(f"      caret col={x} row={y}  {rows[y]!r}")
        time.sleep(2.0)
    watch.close()


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-composer-XXXXXX"],
                         capture_output=True, text=True).stdout.strip()
    start()
    try:
        for harness, cmd, boot, settle in [
            ("claude", "claude", 14, []),
            # Both refuse to open a directory they have not been told to trust, and the answer is
            # a keypress on a menu rather than anything this probe is measuring.
            ("codex", "codex", 14, [("\r", 6)]),
            ("agy", "agy", 18, [("\r", 10)]),
        ]:
            if not shutil.which(harness):
                print(f"\n{harness}: not on PATH, skipped")
                continue
            try:
                run(harness, cmd, boot, cwd, settle)
            except Exception as e:
                import traceback
                print(f"\n!!! {harness} probe failed: {e}")
                traceback.print_exc()
    finally:
        stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
