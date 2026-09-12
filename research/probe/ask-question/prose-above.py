#!/usr/bin/env python3
"""What does the conversation have to work with when Claude writes *prose* and then asks?

#42/#421 both raised a question with nothing above it — "ask nothing else, use no other tool" —
so both measured a frozen transcript and a screen holding only the dialog. The operator's report
is the other shape: the prose and the tool cards leading up to the question are on the terminal
and missing from the conversation pane.

Two readings, one run: the visible screen while the dialog stands (saved as a fixture, so the
`live::read` walk is measured against a real dialog rather than a hand-written one), and every
record on disk at that moment — which separates "the prose was never written down" from "the
prose was written down and the reader did not find it".

Usage: python3 research/probe/ask-question/prose-above.py
"""
import glob, json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(SP, "prose-frames")
NAME = "kampr-probe-prose-%d" % os.getpid()
SOCK = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % NAME)
WORK = os.path.join(SP, "prose-work")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM", "CLAUDE_CODE_CHILD_SESSION")}

PROMPT = ("Do these four things in order and nothing else. One: write me two full sentences of "
          "prose about why integration tests are worth their cost. Two: run the shell command "
          "`echo kampr-probe-prose` with the Bash tool. Three: write me one more full sentence of "
          "prose saying what that command printed. Four: use the AskUserQuestion tool to ask me "
          "which suite to run, with the options unit and integration. Do not write any files.")


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit("%s failed: %s" % (method, r))
    return r["result"]


def start():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    subprocess.Popen(["herdr", "server", "--session", NAME], env=CLEAN_ENV,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(200):
        if os.path.exists(SOCK):
            time.sleep(0.8)
            return
        time.sleep(0.1)
    raise SystemExit("herdr never came up")


def stop():
    try:
        rpc("server.stop", {}, sock_path=SOCK)
    except Exception:
        pass
    time.sleep(1.0)
    shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)


def screen(pane):
    return call("pane.read", {"pane_id": pane, "source": "visible", "format": "text",
                              "strip_ansi": True})["read"]["text"]


def slug(path):
    return "-" + path.strip("/").replace("/", "-").replace("_", "-").replace(".", "-")


def transcripts():
    return glob.glob(os.path.expanduser("~/.claude/projects/%s/*.jsonl" % slug(WORK)))


def records():
    """Every record on disk, flattened to (timestamp, role, kind, first 120 chars)."""
    out = []
    for path in transcripts():
        for line in open(path, encoding="utf-8", errors="ignore"):
            try:
                r = json.loads(line)
            except Exception:
                continue
            msg = r.get("message") or {}
            content = msg.get("content")
            if isinstance(content, str):
                out.append((r.get("timestamp"), msg.get("role"), "text", content[:120]))
                continue
            if not isinstance(content, list):
                continue
            for b in content:
                if not isinstance(b, dict):
                    continue
                kind = b.get("type")
                if kind == "text":
                    head = " ".join(b.get("text", "").split())[:120]
                elif kind == "tool_use":
                    head = "%s %s" % (b.get("name"), json.dumps(b.get("input"))[:80])
                elif kind == "tool_result":
                    head = "for %s" % b.get("tool_use_id")
                else:
                    head = ""
                out.append((r.get("timestamp"), msg.get("role"), kind, head))
    return out


def save(name, text):
    with open(os.path.join(OUT, name + ".txt"), "w") as f:
        f.write(text)


def main():
    os.makedirs(OUT, exist_ok=True)
    start()
    try:
        ws = call("workspace.create", {"label": "prose", "cwd": WORK})
        pane = ws["root_pane"]["pane_id"]
        print("pane %s cwd %s" % (pane, WORK), flush=True)
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": pane, "text": "claude --dangerously-skip-permissions\r"})
        time.sleep(10.0)
        call("pane.send_text", {"pane_id": pane, "text": PROMPT})
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "\r"})

        dialog, t0 = None, time.time()
        while time.time() - t0 < 240:
            text = screen(pane)
            if "Enter to select" in text:
                time.sleep(1.0)
                dialog = screen(pane)
                break
            time.sleep(0.5)
        if dialog is None:
            save("no-dialog", screen(pane))
            print("NO DIALOG", flush=True)
            return
        save("dialog", dialog)
        print("=== VISIBLE SCREEN WHILE THE DIALOG STANDS ===", flush=True)
        print(dialog, flush=True)
        print("=== ON DISK AT THAT MOMENT ===", flush=True)
        for ts, role, kind, head in records():
            print("  %s %-9s %-11s %s" % (ts, role, kind, head), flush=True)

        call("pane.send_text", {"pane_id": pane, "text": "1"})
        time.sleep(8.0)
        save("answered", screen(pane))
        print("=== ON DISK AFTER ANSWERING ===", flush=True)
        for ts, role, kind, head in records():
            print("  %s %-9s %-11s %s" % (ts, role, kind, head), flush=True)
        for path in transcripts():
            shutil.copy(path, os.path.join(OUT, os.path.basename(path)))
    finally:
        stop()
        shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
