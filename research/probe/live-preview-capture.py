#!/usr/bin/env python3
"""Every screen the live preview is read from, through one real streamed Claude turn.

The node polls the pane's grid every 200 ms (`convo::LIVE_POLL`) and hands each screen to
`kampr_journal::live`. This keeps the same cadence against a real `claude` in a throwaway named
herdr session, asked for an answer longer than the pane, and writes every distinct screen with its
time — plus the time each assistant record lands in the transcript — to a JSON file, so the
node's own `Watch` can be replayed over exactly what a reader was shown.

Usage: live-preview-capture.py OUT.json [ROWS]
"""
import glob, json, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-livecap-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
composer.COLS = 200
if len(sys.argv) > 2:
    composer.ROWS = int(sys.argv[2])
# A probe run from inside Claude Code inherits a marker that turns the child's transcript off.
for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]

ASK = (
    "Explain in plain prose, with no tools, no headings and no lists, how a TCP connection is "
    "opened, kept alive and closed. Write six paragraphs of five or six sentences each."
)


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def transcript(cwd):
    slug = cwd.replace("/", "-").replace(".", "-")
    found = glob.glob(os.path.expanduser(f"~/.claude/projects/{slug}/*.jsonl"))
    return max(found, key=os.path.getmtime) if found else None


def records(path, seen):
    out = []
    if not path:
        return out
    with open(path) as f:
        for line in f.readlines()[seen:]:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            msg = rec.get("message") or {}
            if rec.get("type") != "assistant":
                out.append(None)
                continue
            texts = [b.get("text", "") for b in msg.get("content", []) if b.get("type") == "text"]
            out.append("\n".join(texts) if texts else None)
    return out


def main():
    out_path = sys.argv[1]
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-livecap-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        for _ in range(60):
            time.sleep(0.5)
            rows = watch.look()[0]
            if any("trust" in r.lower() for r in rows):
                send(pane, "\r")
                time.sleep(4)
                break
            if any(r.startswith("❯") for r in rows):
                break
        send(pane, ASK)
        time.sleep(0.5)
        send(pane, "\r")

        t0 = time.time()
        screens, recs, seen_lines = [], [], 0
        quiet = 0
        while time.time() - t0 < 120:
            rows = watch.look()[0]
            now = round(time.time() - t0, 3)
            if not screens or screens[-1]["rows"] != rows:
                screens.append({"t": now, "rows": rows})
                quiet = 0
            else:
                quiet += 1
            path = transcript(cwd)
            new = records(path, seen_lines)
            for text in new:
                seen_lines += 1
                if text:
                    recs.append({"t": now, "text": text})
            if recs and quiet > 25:
                break
            time.sleep(0.2)
        with open(out_path, "w") as f:
            json.dump({"cols": composer.COLS, "rows": composer.ROWS, "screens": screens, "records": recs}, f)
        print(f"{len(screens)} distinct screens, {len(recs)} text records -> {out_path}")
        for _ in range(3):
            send(pane, "\x03")
            time.sleep(0.3)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
