#!/usr/bin/env python3
"""What `omp` asks with, and what answers it.

The question Kampr most wants to put on a phone is the one a pane is blocked on, and answering it
is a keystroke a client has to be *told*: a digit answers Claude's single-answer question and ticks
a box on its multiple-answer one (#413, #421), and a harness that draws no numbers at all cannot be
answered by either. This measures omp's two blocking dialogs — a tool approval and the `ask` tool —
and tries each candidate keystroke against a real one.

Runs in a throwaway named herdr session, torn down at the end (#97).

    bun research/probe/omp-mock-anthropic.ts &      # port 8899
    research/probe/omp-dialog.py
"""
import argparse, base64, json, os, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import vt

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-ompask-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
COLS, ROWS = 95, 40
ENDPOINT = os.environ.get("OMP_MOCK", "http://127.0.0.1:8899")
OMP = os.environ.get("OMP_BIN", os.path.expanduser("~/.bun/bin/omp"))
BUN = os.environ.get("OMP_BUN_DIR", "")


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
            with self.lock:
                vt.feed(self.screen, base64.b64decode(rec["bytes"]).decode("utf-8", "replace"))

    def look(self):
        with self.lock:
            return ["".join(r).rstrip() for r in self.screen.g], self.screen.x, self.screen.y

    def close(self):
        self.child.kill()


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def dialog(rows):
    """The dialog's own rows, if one is on the screen."""
    for i, row in enumerate(rows):
        if row.startswith("╭─ ") and ("Allow tool" in row or "?" in row or "ask" in row.lower()):
            end = next((j for j in range(i + 1, len(rows)) if rows[j].startswith("╰")), len(rows) - 1)
            return rows[i:end + 1]
    return None


def wait_for_dialog(watch, seconds=25):
    for _ in range(seconds * 2):
        rows, _, _ = watch.look()
        found = dialog(rows)
        if found:
            return found
        time.sleep(0.5)
    return None


def read_visible(pane):
    """The same read `pending::read` makes, so a fixture is the production path's own input."""
    r = call("pane.read", {"pane_id": pane, "source": "visible", "format": "text", "strip_ansi": True})
    return r["read"]["text"]


def save(out, name, text):
    if out is None:
        return
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, f"{name}.txt"), "w") as f:
        f.write(text if text.endswith("\n") else text + "\n")
    print(f"    wrote {name}.txt")


def status(pane):
    return call("pane.get", {"pane_id": pane})["pane"].get("agent_status")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None, help="directory to write dialog fixtures into")
    args = ap.parse_args()
    if not os.path.exists(OMP):
        raise SystemExit(f"no omp at {OMP} (set OMP_BIN)")
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-ompask-XXXXXX"], capture_output=True, text=True).stdout.strip()
    start()
    try:
        ws = call("workspace.create", {"label": "ompask", "cwd": cwd})
        pane = ws["root_pane"]["pane_id"]
        watch = Watch(pane)
        time.sleep(1.0)
        path = f"{BUN}:{os.path.dirname(OMP)}:$PATH" if BUN else f"{os.path.dirname(OMP)}:$PATH"
        send(pane, f"ANTHROPIC_BASE_URL={ENDPOINT} ANTHROPIC_API_KEY=probe PATH={path} "
                   f"{OMP} --model claude-sonnet-4-5 --no-lsp --approval-mode always-ask\r")
        time.sleep(14)
        for _ in range(6):
            send(pane, "\x1b")
            time.sleep(1.0)

        for label, prompt, keys, fixture in [
            ("tool approval, a digit", "SLOWCMD approve me\r", ["1"], "omp-approval"),
            ("tool approval, one arrow down", "SLOWCMD approve me again\r", ["\x1b[B"], "omp-approval-moved"),
            ("the ask tool, a digit", "ASKME which branch\r", ["1"], "omp-ask"),
            ("the ask tool, enter", "ASKME which branch again\r", ["\r"], None),
            # Several answers on one question, and a second question behind it.
            ("ask, several answers", "ASKTWO decide both\r", [" "], "omp-ask-multi"),
            ("ask, committing them", "", ["\x1b[B", " ", "\r"], "omp-ask-multi-ticked"),
            ("ask, the second question", "", [], "omp-ask-second"),
        ]:
            print(f"\n=== {label}")
            if prompt:
                send(pane, prompt)
            found = wait_for_dialog(watch)
            if not found:
                rows, _, _ = watch.look()
                print("  no dialog; screen tail:")
                for r in rows[-8:]:
                    print(f"    {r!r}")
                continue
            print(f"  herdr says: {status(pane)}")
            for r in found:
                print(f"    {r!r}")
            if fixture and not fixture.endswith("moved"):
                save(args.out, fixture, read_visible(pane))
            for key in keys:
                send(pane, key)
                time.sleep(1.5)
                rows, _, _ = watch.look()
                still = dialog(rows)
                print(f"  after {key!r}: dialog {'still open' if still else 'GONE'}")
                if still and still != found:
                    for r in still:
                        print(f"      {r!r}")
                    found = still
                    if fixture and fixture.endswith("moved"):
                        save(args.out, fixture, read_visible(pane))
                if not still:
                    break
            if fixture and fixture.startswith("omp-ask-"):
                # These cases walk one dialog forward, so nothing is closed between them — and the
                # state a dialog *opens* in is the one that says what kind of question it is, so it
                # is kept beside the state the keys left it in.
                save(args.out, f"{fixture}-ticked", read_visible(pane))
                continue
            # Esc closes whatever is still standing, so the next question opens on a clean pane.
            send(pane, "\x1b")
            time.sleep(2)
            # Leave the pane idle before the next question.
            for _ in range(20):
                if status(pane) != "working":
                    break
                time.sleep(1)
        watch.close()
    finally:
        stop()


if __name__ == "__main__":
    main()
