#!/usr/bin/env python3
"""What `omp` (oh-my-pi) paints: the message it is writing, the composer, and what clears it.

Runs in a throwaway named herdr session of its own, torn down at the end (#97: a node serves every
session it can find). The screen and the caret come off a real `terminal session observe` stream
through research/probe/vt.py, which is the grid Kampr's own emulator builds — so the caret column
measured here is the one `PaneRegistry` would report.

**omp is driven against `omp-mock-anthropic.ts`, not against a provider.** The harness takes its
real code paths — streaming deltas, tool calls, spawns — with no credentials and no network, and
the answer it streams is long enough on purpose: a preview is only published for a block that
*grows* between polls, so a one-shot reply cannot exercise the reader that reads it.

    bun research/probe/omp-mock-anthropic.ts &      # port 8899
    research/probe/omp-screens.py [--out <dir>]

Writes the fixtures the `omp` composer and live tests read, in their own format: a `caret <col>
<row>` header for a composer capture, and a bare grid for a screen.
"""
import argparse, base64, json, os, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import vt

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-omp-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
COLS, ROWS = 95, 40
ENDPOINT = os.environ.get("OMP_MOCK", "http://127.0.0.1:8899")
OMP = os.environ.get("OMP_BIN", os.path.expanduser("~/.bun/bin/omp"))
# omp 18 needs a bun newer than this machine's package manager ships; `bun.sh/install` into a
# scratch prefix is what the probe used.
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


def save(out, name, rows, caret=None):
    if out is None:
        return
    os.makedirs(out, exist_ok=True)
    body = "\n".join(rows).rstrip("\n") + "\n"
    head = f"caret {caret[0]} {caret[1]}\n" if caret else ""
    with open(os.path.join(out, f"{name}.txt"), "w") as f:
        f.write(head + body)
    print(f"    wrote {name}.txt")


def show(rows, y, span=4):
    for i in range(max(0, y - span), min(len(rows), y + 2)):
        print(f"      [{i}]{'*' if i == y else ' '}{rows[i]!r}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None, help="directory to write fixtures into")
    args = ap.parse_args()

    if not os.path.exists(OMP):
        raise SystemExit(f"no omp at {OMP} (set OMP_BIN)")
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-omp-XXXXXX"], capture_output=True, text=True).stdout.strip()
    with open(os.path.join(cwd, "README.md"), "w") as f:
        f.write("hello world\n")
    start()
    try:
        ws = call("workspace.create", {"label": "omp", "cwd": cwd})
        pane = ws["root_pane"]["pane_id"]
        watch = Watch(pane)
        time.sleep(1.0)
        path = f"{BUN}:{os.path.dirname(OMP)}:$PATH" if BUN else f"{os.path.dirname(OMP)}:$PATH"
        send(pane, f"ANTHROPIC_BASE_URL={ENDPOINT} ANTHROPIC_API_KEY=probe PATH={path} "
                   f"{OMP} --model claude-sonnet-4-5 --auto-approve --no-lsp\r")
        time.sleep(14)
        # The first run walks a five-step onboarding; Esc skips each of them.
        for _ in range(6):
            send(pane, "\x1b")
            time.sleep(1.0)
        time.sleep(2)
        rows, x, y = watch.look()
        print("  EMPTY composer:")
        show(rows, y)
        save(args.out, "omp-empty", rows, (x, y))

        typed = "push the branch when the tests go green"
        send(pane, typed)
        time.sleep(1.5)
        rows, x, y = watch.look()
        print(f"  after send_text({typed!r}):")
        show(rows, y)
        save(args.out, "omp-typed", rows, (x, y))

        send(pane, "\x01")
        time.sleep(1.0)
        rows, x, y = watch.look()
        print("  caret sent home with the line still full (ctrl+a):")
        show(rows, y, 2)
        save(args.out, "omp-caret-at-home", rows, (x, y))
        send(pane, "\x05")
        time.sleep(0.8)

        print("  --- what clears it")
        for label, keys in [("ctrl+u (\\x15)", "\x15"), ("ctrl+a ctrl+k (\\x01\\x0b)", "\x01\x0b")]:
            rows, _, _ = watch.look()
            before = sum(1 for r in rows if typed in r)
            send(pane, keys)
            time.sleep(1.4)
            rows, x, y = watch.look()
            after = sum(1 for r in rows if typed in r)
            print(f"    {label}: lines carrying the typed text {before} -> {after}; caret row {rows[y]!r}")
            if after:
                send(pane, "\x15")
                time.sleep(1.0)
            else:
                send(pane, typed)
                time.sleep(1.2)
        send(pane, "\x15")
        time.sleep(1.0)

        print("  --- a line too long for one row, to see how a wrapped composer paints")
        send(pane, typed + " and then rebase it onto main, run the whole gate twice, "
                           "and tell me which of the two runs was the slower one")
        time.sleep(1.8)
        rows, x, y = watch.look()
        show(rows, y, 6)
        save(args.out, "omp-wrapped", rows, (x, y))
        send(pane, "\x15")
        time.sleep(1.0)

        print("  --- the message it is painting, sampled while it streams")
        send(pane, "LONGANSWER please explain it\r")
        for n in range(1, 9):
            time.sleep(1.6)
            rows, x, y = watch.look()
            print(f"    [{n}] caret row {y}")
            show(rows, y, 8)
            save(args.out, f"omp-streaming-{n}", rows)

        print("  --- prompts typed while it is still working")
        time.sleep(2)
        send(pane, "and then push it")
        time.sleep(1.2)
        rows, x, y = watch.look()
        show(rows, y, 3)
        save(args.out, "omp-queued-typed", rows, (x, y))
        send(pane, "\r")
        time.sleep(2.0)
        rows, x, y = watch.look()
        print("    after submitting it:")
        show(rows, y, 6)
        save(args.out, "omp-queued-sent", rows, (x, y))
        # A second one, to see whether the list numbers and how it grows.
        send(pane, "and tag the release too")
        time.sleep(1.2)
        send(pane, "\r")
        time.sleep(2.0)
        rows, x, y = watch.look()
        print("    with two waiting:")
        show(rows, y, 8)
        save(args.out, "omp-queued-two", rows, (x, y))
        # And one too long for a row, to see whether a queued prompt wraps and where.
        send(pane, "and then write the release notes, mentioning every probe row this work added "
                   "and which of them changed a decision")
        time.sleep(1.2)
        send(pane, "\r")
        time.sleep(2.0)
        rows, x, y = watch.look()
        print("    with a long one waiting:")
        show(rows, y, 10)
        save(args.out, "omp-queued-wrapped", rows, (x, y))
        time.sleep(20)
        rows, x, y = watch.look()
        print("    once the turn ended:")
        show(rows, y, 8)
        save(args.out, "omp-queued-after", rows, (x, y))
        watch.close()
        print(f"\n  session files under ~/.omp/agent/sessions for cwd {cwd}")
    finally:
        stop()


if __name__ == "__main__":
    main()
