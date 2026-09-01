#!/usr/bin/env python3
"""Does the bare digit still answer an AskUserQuestion when the dialog's pane is NOT the
session-focused one? Kampr never focuses a pane (rule 3), so this is the shape every answer sent
from a phone actually has.
"""
import json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-askq-unfocused-%d" % os.getpid()
SOCK = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % NAME)
OUT = os.path.join(SP, "unfocused-frames")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM", "CLAUDE_CODE_CHILD_SESSION")}

PROMPT = ("Use the AskUserQuestion tool right now to ask me which indentation I prefer, with the "
          "options Tabs, Two spaces and Four spaces. Ask nothing else, use no other tool, and do "
          "not write any files.")


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit("%s failed: %s" % (method, r))
    return r["result"]


def start():
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
    return call("pane.read", {"pane_id": pane, "source": "visible",
                              "format": "text", "strip_ansi": True})["read"]["text"]


def save(name, text):
    with open(os.path.join(OUT, name + ".txt"), "w") as f:
        f.write(text)
    print("=== %s ===\n%s" % (name, text), flush=True)


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(OUT)
    start()
    try:
        work = os.path.join(SP, "work")
        os.makedirs(work, exist_ok=True)
        ws = call("workspace.create", {"label": "askq", "cwd": work})
        agent = ws["root_pane"]["pane_id"]
        print("agent pane", agent, "session", NAME, flush=True)
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": agent, "text": "claude --dangerously-skip-permissions\r"})
        time.sleep(10.0)
        call("pane.send_text", {"pane_id": agent, "text": PROMPT})
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": agent, "text": "\r"})

        t0 = time.time()
        while time.time() - t0 < 120:
            if "Enter to select" in screen(agent):
                break
            time.sleep(0.5)
        else:
            save("no-dialog", screen(agent))
            raise SystemExit("no dialog")
        time.sleep(1.0)
        save("01-dialog", screen(agent))

        # A second pane, focused, so the dialog's pane is in the background exactly as it is when
        # an operator answers from a phone.
        split = call("pane.split", {"pane_id": agent, "direction": "right"})
        other = split.get("pane", split).get("pane_id") if isinstance(split, dict) else None
        print("split reply:", json.dumps(split)[:300], flush=True)
        if other:
            call("pane.focus", {"pane_id": other})
        time.sleep(1.5)
        info = call("pane.get", {"pane_id": agent})
        print("agent pane focused:", info["pane"]["focused"], flush=True)

        call("pane.send_text", {"pane_id": agent, "text": "2"})
        time.sleep(4.0)
        after = screen(agent)
        save("02-after-bare-digit-unfocused", after)
        print("ANSWERED WHILE UNFOCUSED:", "Enter to select" not in after, flush=True)
    finally:
        stop()


if __name__ == "__main__":
    main()
