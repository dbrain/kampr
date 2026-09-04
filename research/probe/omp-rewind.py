#!/usr/bin/env python3
"""What a rewind leaves in an `omp` session file, and what a reader that walks it in file order
would then say the agent said.

**omp's session is a tree.** `docs/tree.md`: `/tree`, `/branch` and the double-Escape selector move
a *leaf pointer*, and "the old answer branch is preserved". Every entry carries `id`/`parentId` and
`buildSessionContext` walks parents from the leaf — so a reader that takes the file in order is
reading turns the operator took back. This drives one, prints the tree, and prints both readings:
the file's order, and the root-to-leaf path.

Runs in a throwaway named herdr session, torn down at the end (#97).

    bun research/probe/omp-mock-anthropic.ts &      # port 8899
    research/probe/omp-rewind.py
"""
import base64, json, os, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import vt

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-omprewind-{os.getpid()}"
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


def show(rows, y, span=12):
    for i in range(max(0, y - span), min(len(rows), y + 2)):
        print(f"      [{i}]{'*' if i == y else ' '}{rows[i]!r}")


def transcript(cwd):
    root = os.path.expanduser("~/.omp/agent/sessions")
    found = []
    for bucket in os.listdir(root):
        d = os.path.join(root, bucket)
        if not os.path.isdir(d):
            continue
        for name in os.listdir(d):
            p = os.path.join(d, name)
            if name.endswith(".jsonl") and os.path.isfile(p):
                with open(p) as f:
                    head = f.read(4096)
                if f'"cwd":"{cwd}"' in head.replace(" ", ""):
                    found.append((os.path.getmtime(p), p))
    found.sort()
    return found[-1][1] if found else None


def entries(path):
    out = []
    with open(path) as f:
        for line in f:
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def spoken(entry):
    if entry.get("type") != "message":
        return None
    message = entry["message"]
    content = message.get("content")
    if not isinstance(content, list):
        return None
    text = " ".join(c.get("text", "") for c in content if c.get("type") == "text").strip()
    return f'{message.get("role")}: {text[:60]}' if text else None


def main():
    if not os.path.exists(OMP):
        raise SystemExit(f"no omp at {OMP} (set OMP_BIN)")
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-omprewind-XXXXXX"],
                         capture_output=True, text=True).stdout.strip()
    start()
    try:
        ws = call("workspace.create", {"label": "omprewind", "cwd": cwd})
        pane = ws["root_pane"]["pane_id"]
        watch = Watch(pane)
        time.sleep(1.0)
        path = f"{BUN}:{os.path.dirname(OMP)}:$PATH" if BUN else f"{os.path.dirname(OMP)}:$PATH"
        send(pane, f"ANTHROPIC_BASE_URL={ENDPOINT} ANTHROPIC_API_KEY=probe PATH={path} "
                   f"{OMP} --model claude-sonnet-4-5 --auto-approve --no-lsp\r")
        time.sleep(14)
        for _ in range(6):
            send(pane, "\x1b")
            time.sleep(1.0)

        for word in ["alpha", "bravo"]:
            send(pane, f"SAY: {word}\r")
            time.sleep(8)
            rows, _, y = watch.look()
            print(f"  after {word}: {rows[max(0, y - 4)]!r}")

        print("\n  --- /tree")
        send(pane, "/tree\r")
        time.sleep(3.0)
        rows, x, y = watch.look()
        show(rows, y, 20)

        # Search is the deterministic way in: typing filters the tree, so the first prompt is
        # selected by naming it rather than by counting rows.
        send(pane, "alpha")
        time.sleep(1.5)
        rows, x, y = watch.look()
        print("\n  --- with the tree searched for the first prompt")
        show(rows, y, 20)
        send(pane, "\r")
        time.sleep(3)
        rows, x, y = watch.look()
        print("\n  --- after enter")
        show(rows, y, 16)

        send(pane, "SAY: charlie\r")
        time.sleep(10)
        rows, x, y = watch.look()
        print("\n  --- after the third prompt")
        show(rows, y, 16)
        watch.close()

        found = transcript(cwd)
        print(f"\n  transcript: {found}")
        if not found:
            return
        rows = entries(found)
        by_id = {e.get("id"): e for e in rows if e.get("id")}
        print("\n  --- the file, in order")
        for e in rows:
            said = spoken(e)
            print(f"    {e.get('type'):16} id={e.get('id')} parent={e.get('parentId')}"
                  + (f"  {said}" if said else ""))
        leaf = rows[-1]
        print(f"\n  --- root-to-leaf from {leaf.get('id')}")
        path_ids, at, seen = [], leaf, set()
        while at is not None and at.get("id") not in seen:
            seen.add(at.get("id"))
            path_ids.append(at.get("id"))
            at = by_id.get(at.get("parentId"))
        for entry_id in reversed(path_ids):
            said = spoken(by_id.get(entry_id, {}))
            if said:
                print(f"    {said}")
        print("\n  --- what a reader in file order would say instead")
        for e in rows:
            said = spoken(e)
            if said:
                print(f"    {said}")
    finally:
        stop()


if __name__ == "__main__":
    main()
