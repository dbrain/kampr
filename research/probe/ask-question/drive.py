#!/usr/bin/env python3
"""Does a bare digit answer Claude Code's AskUserQuestion dialog, the way it answers a
permission prompt (#72, #106)?

Throwaway named herdr session, torn down. Raises a real AskUserQuestion, sends the bare key,
reads the screen, then sends Enter and reads it again.
"""
import json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-askq-%d" % os.getpid()
SOCK = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % NAME)
OUT = os.path.join(SP, "frames")

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


def wait_for_dialog(pane, seconds):
    t0 = time.time()
    while time.time() - t0 < seconds:
        s = screen(pane)
        if "Enter to select" in s or ("1. " in s and "2. " in s and "3. " in s):
            time.sleep(1.0)
            return screen(pane)
        time.sleep(0.5)
    return None


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(OUT)
    start()
    try:
        work = os.path.join(SP, "work")
        os.makedirs(work, exist_ok=True)
        ws = call("workspace.create", {"label": "askq", "cwd": work})
        pane = ws["root_pane"]["pane_id"]
        print("pane", pane, "session", NAME, flush=True)
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": pane, "text": "claude --dangerously-skip-permissions\r"})
        time.sleep(10.0)
        save("00-launched", screen(pane))
        call("pane.send_text", {"pane_id": pane, "text": PROMPT})
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "\r"})

        dialog = wait_for_dialog(pane, 120)
        if dialog is None:
            save("01-no-dialog", screen(pane))
            raise SystemExit("no AskUserQuestion dialog appeared")
        save("01-dialog", dialog)
        print("agent state:", json.dumps(call("pane.get", {"pane_id": pane})), flush=True)

        call("pane.send_text", {"pane_id": pane, "text": "2"})
        time.sleep(3.0)
        after_digit = screen(pane)
        save("02-after-bare-digit", after_digit)
        print("SUBMITTED BY DIGIT:", "Enter to select" not in after_digit, flush=True)

        call("pane.send_text", {"pane_id": pane, "text": "\r"})
        time.sleep(3.0)
        save("03-after-enter", screen(pane))
    finally:
        stop()


if __name__ == "__main__":
    main()
