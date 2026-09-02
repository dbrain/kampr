#!/usr/bin/env python3
"""What answers a multi-select AskUserQuestion, given that a bare digit does not?

`on-disk.py multi` sent `1` and the tool never completed — 0 calls recorded after six seconds —
which is the opposite of #413's reading for the single-select dialog, where a bare digit took
effect on its own. A multi-select draws `[ ]` against each option, so a digit is a *toggle*. This
sends two toggles and then Enter, and reads the transcript to see whether the tool completed and
with what.
"""
import glob, json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(SP, "on-disk-frames")
NAME = "kampr-probe-multians-%d" % os.getpid()
SOCK = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % NAME)
WORK = os.path.join(SP, "multi-answer-work")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM", "CLAUDE_CODE_CHILD_SESSION")}

PROMPT = ("Use the AskUserQuestion tool right now to ask me, with multiSelect true, which of these "
          "test suites to run: unit, integration, browser. Ask nothing else, use no other tool, "
          "and do not write any files.")


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit("%s failed: %s" % (method, r))
    return r["result"]


def slug(path):
    return "-" + path.strip("/").replace("/", "-").replace("_", "-").replace(".", "-")


def answered():
    out = []
    for path in glob.glob(os.path.expanduser("~/.claude/projects/%s/*.jsonl" % slug(WORK))):
        for line in open(path, encoding="utf-8", errors="ignore"):
            try:
                r = json.loads(line)
            except Exception:
                continue
            tur = r.get("toolUseResult")
            if isinstance(tur, dict) and "questions" in tur:
                out.append(tur)
    return out


def screen(pane):
    return call("pane.read", {"pane_id": pane, "source": "visible",
                              "format": "text", "strip_ansi": True})["read"]["text"]


def save(name, text):
    with open(os.path.join(OUT, name + ".txt"), "w") as f:
        f.write(text)


def main():
    os.makedirs(OUT, exist_ok=True)
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    subprocess.Popen(["herdr", "server", "--session", NAME], env=CLEAN_ENV,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(200):
        if os.path.exists(SOCK):
            time.sleep(0.8)
            break
        time.sleep(0.1)
    try:
        ws = call("workspace.create", {"label": "multians", "cwd": WORK})
        pane = ws["root_pane"]["pane_id"]
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": pane, "text": "claude --dangerously-skip-permissions\r"})
        time.sleep(10.0)
        call("pane.send_text", {"pane_id": pane, "text": PROMPT})
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "\r"})

        t0 = time.time()
        while time.time() - t0 < 150:
            if "Enter to select" in screen(pane):
                time.sleep(1.0)
                break
            time.sleep(0.5)
        else:
            raise SystemExit("no dialog")
        save("multians-00-dialog", screen(pane))

        # Two toggles, from a pane nobody focused — which is how an answer from a phone arrives.
        for key in ("1", "3"):
            call("pane.send_text", {"pane_id": pane, "text": key})
            time.sleep(1.5)
        after = screen(pane)
        save("multians-01-after-two-digits", after)
        print("after two digits, tool completed:", len(answered()), flush=True)
        print("checkbox rows:", [l for l in after.splitlines() if "[" in l and "]" in l], flush=True)

        call("pane.send_text", {"pane_id": pane, "text": "\r"})
        time.sleep(4.0)
        save("multians-02-after-enter", screen(pane))
        print("after Enter, tool completed:", len(answered()), flush=True)

        # Enter toggled the focused row instead of submitting, and the header row carries
        # `<-  [ ] Test suites  |/ Submit  ->`. So the submit is a *tab*, reached sideways.
        for label, keys in (("right-arrow", "\x1b[C"), ("tab", "\t")):
            call("pane.send_text", {"pane_id": pane, "text": keys})
            time.sleep(1.5)
            save("multians-03-after-%s" % label, screen(pane))
            call("pane.send_text", {"pane_id": pane, "text": "\r"})
            time.sleep(6.0)
            save("multians-04-after-%s-enter" % label, screen(pane))
            got = answered()
            print("after %s + Enter, tool completed: %d" % (label, len(got)), flush=True)
            if got:
                for g in got:
                    print(json.dumps(g)[:700], flush=True)
                break
    finally:
        try:
            rpc("server.stop", {}, sock_path=SOCK)
        except Exception:
            pass
        time.sleep(1.0)
        shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)
        shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
