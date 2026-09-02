#!/usr/bin/env python3
"""Is Claude Code's question on disk *while it is asking*, or only after it is answered?

Probe #42 measured the second — "the file froze for 4m20s at the prompt and only then jumped to
carry both the `tool_use` and its result" — and every pending question Kampr has ever published has
been scraped off the pane's screen because of it. That was a long time ago and `AskUserQuestion` did
not exist then. The whole of whether a client can render a question *properly* — its header, its
per-option descriptions, whether it takes several answers — turns on this one reading, because none
of that is on the screen to scrape.

Two runs, each a throwaway named herdr session torn down at the end:

  ask         raise a real AskUserQuestion, and poll the transcript while the dialog stands
  permission  raise a real Bash permission prompt (no --dangerously-skip-permissions), same poll

Usage: python3 research/probe/ask-question/on-disk.py [ask|permission|both]
"""
import glob, json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(SP, "on-disk-frames")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM", "CLAUDE_CODE_CHILD_SESSION")}

ASK = ("Use the AskUserQuestion tool right now to ask me which indentation I prefer, with the "
       "options Tabs, Two spaces and Four spaces. Ask nothing else, use no other tool, and do "
       "not write any files.")

PERMISSION = "Run the shell command `echo kampr-probe-permission` with the Bash tool. Nothing else."

# The other half of the shape: a question that takes several answers at once. Rendering one as if
# it took a single answer is worse than not rendering it, because a digit *toggles* rather than
# answers and the pane would sit there looking ignored.
MULTI = ("Use the AskUserQuestion tool right now to ask me, with multiSelect true, which of these "
         "test suites to run: unit, integration, browser. Ask nothing else, use no other tool, and "
         "do not write any files.")


class Session:
    def __init__(self, tag):
        self.name = "kampr-probe-ondisk-%s-%d" % (tag, os.getpid())
        self.sock = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % self.name)
        self.work = os.path.join(SP, "on-disk-work-%s" % tag)

    def call(self, method, params=None):
        r = rpc(method, params or {}, sock_path=self.sock)
        if not r or "error" in r:
            raise SystemExit("%s failed: %s" % (method, r))
        return r["result"]

    def start(self):
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)
        subprocess.Popen(["herdr", "server", "--session", self.name], env=CLEAN_ENV,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
        for _ in range(200):
            if os.path.exists(self.sock):
                time.sleep(0.8)
                return
            time.sleep(0.1)
        raise SystemExit("herdr never came up")

    def stop(self):
        try:
            rpc("server.stop", {}, sock_path=self.sock)
        except Exception:
            pass
        time.sleep(1.0)
        shutil.rmtree(os.path.dirname(self.sock), ignore_errors=True)

    def screen(self, pane):
        return self.call("pane.read", {"pane_id": pane, "source": "visible",
                                       "format": "text", "strip_ansi": True})["read"]["text"]


def slug(path):
    return "-" + path.strip("/").replace("/", "-").replace("_", "-").replace(".", "-")


def transcripts(work):
    return glob.glob(os.path.expanduser("~/.claude/projects/%s/*.jsonl" % slug(work)))


def calls_on_disk(work):
    """Every tool_use in whatever transcript this cwd has, with the ids that have results."""
    found, settled = [], set()
    for path in transcripts(work):
        for line in open(path, encoding="utf-8", errors="ignore"):
            try:
                r = json.loads(line)
            except Exception:
                continue
            c = (r.get("message") or {}).get("content")
            if not isinstance(c, list):
                continue
            for b in c:
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "tool_use":
                    found.append((r.get("timestamp"), b.get("name"), b.get("id"), b.get("input")))
                elif b.get("type") == "tool_result":
                    settled.add(b.get("tool_use_id"))
    return found, settled


def save(name, text):
    with open(os.path.join(OUT, name + ".txt"), "w") as f:
        f.write(text)


def wait_for(pane, s, marks, seconds):
    t0 = time.time()
    while time.time() - t0 < seconds:
        text = s.screen(pane)
        if any(m in text for m in marks):
            time.sleep(1.0)
            return s.screen(pane)
        time.sleep(0.5)
    return None


def run(tag, prompt, argv, marks, answer):
    s = Session(tag)
    s.start()
    try:
        ws = s.call("workspace.create", {"label": tag, "cwd": s.work})
        pane = ws["root_pane"]["pane_id"]
        print("[%s] pane %s cwd %s" % (tag, pane, s.work), flush=True)
        time.sleep(1.5)
        s.call("pane.send_text", {"pane_id": pane, "text": argv + "\r"})
        time.sleep(10.0)
        s.call("pane.send_text", {"pane_id": pane, "text": prompt})
        time.sleep(2.0)
        s.call("pane.send_text", {"pane_id": pane, "text": "\r"})

        dialog = wait_for(pane, s, marks, 150)
        if dialog is None:
            save("%s-no-dialog" % tag, s.screen(pane))
            print("[%s] NO DIALOG" % tag, flush=True)
            return
        save("%s-dialog" % tag, dialog)
        asked_at = time.time()

        # The reading. Poll for as long as a person might sit in front of the dialog.
        seen = None
        for _ in range(60):
            found, settled = calls_on_disk(s.work)
            open_calls = [c for c in found if c[2] not in settled]
            if open_calls:
                seen = (time.time() - asked_at, open_calls)
                break
            time.sleep(1.0)

        print("[%s] WHILE THE DIALOG STANDS:" % tag, flush=True)
        if seen is None:
            found, _ = calls_on_disk(s.work)
            print("  nothing unanswered on disk after 60 s; %d call(s) recorded at all"
                  % len(found), flush=True)
        else:
            waited, calls = seen
            print("  on disk %.1f s after the dialog appeared:" % waited, flush=True)
            for ts, name, _id, inp in calls:
                print("    %s %s" % (ts, name), flush=True)
                print("    input: %s" % json.dumps(inp)[:900], flush=True)

        s.call("pane.send_text", {"pane_id": pane, "text": answer})
        time.sleep(6.0)
        save("%s-answered" % tag, s.screen(pane))
        found, settled = calls_on_disk(s.work)
        print("[%s] AFTER ANSWERING: %d call(s), %d settled" % (tag, len(found), len(settled)),
              flush=True)
        for ts, name, cid, inp in found:
            print("    %s %s settled=%s" % (ts, name, cid in settled), flush=True)
            print("    input: %s" % json.dumps(inp)[:1400], flush=True)
        for path in transcripts(s.work):
            shutil.copy(path, os.path.join(OUT, "%s-%s" % (tag, os.path.basename(path))))
    finally:
        s.stop()
        shutil.rmtree(s.work, ignore_errors=True)


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "both"
    os.makedirs(OUT, exist_ok=True)
    if which in ("ask", "both"):
        run("ask", ASK, "claude --dangerously-skip-permissions",
            ["Enter to select", "1. "], "1")
    if which in ("multi", "both"):
        run("multi", MULTI, "claude --dangerously-skip-permissions",
            ["Enter to select", "1. "], "1")
    if which in ("permission", "both"):
        run("permission", PERMISSION, "claude", ["Do you want to", "1. Yes"], "1")


if __name__ == "__main__":
    main()
