#!/usr/bin/env python3
"""Where pi's /model and /thinking selectors sit on the screen, and where the caret goes.

The phone report: opening either one locks the bottom of the view to the text entry line and the
options are not visible at all. This measures the half Kampr can see — the grid and the caret off
a real `terminal session observe` stream through research/probe/vt.py, the same grid the node's
emulator builds — so the client's band can be computed against the measured rows.

Runs in a throwaway named herdr session of its own, torn down at the end (#97).
"""
import base64, json, os, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import vt

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-piselt-{os.getpid()}"
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


def nonblank(rows):
    return [i for i, r in enumerate(rows) if r.strip()]


def report(watch, label):
    rows, x, y = watch.look()
    filled = nonblank(rows)
    end = filled[-1] if filled else -1
    print(f"\n=== {label} ===")
    print(f"caret col={x} row={y}   caret row reads {rows[y]!r}")
    print(f"last non-blank row = {end}")
    for i, r in enumerate(rows):
        mark = "*" if i == y else (" " if r.strip() else ".")
        print(f"  [{i:2d}]{mark} {r!r}")
    return rows, x, y, end


def main():
    start()
    try:
        ws = call("workspace.create", {"label": "pi", "cwd": "/tmp"})
        pane = ws["root_pane"]["pane_id"]
        watch = Watch(pane)
        send(pane, "pi\n")
        # pi boots slow (managed tools, model catalog); wait for the composer to appear.
        for _ in range(120):
            rows, _, _ = watch.look()
            if any(len(r.strip()) >= 60 and set(r.strip()) == {"\u2500"} for r in rows):
                break
            time.sleep(0.5)
        else:
            raise SystemExit("pi never showed its composer")
        time.sleep(1.0)
        report(watch, "idle, composer up")

        send(pane, "/model\r")
        time.sleep(2.0)
        report(watch, "/model open")

        send(pane, "\x1b")
        time.sleep(1.0)
        report(watch, "escaped back to composer")

        send(pane, "/thinking\r")
        time.sleep(2.0)
        report(watch, "/thinking open")
        watch.close()
    finally:
        stop()


if __name__ == "__main__":
    main()