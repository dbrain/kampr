#!/usr/bin/env python3
"""What does Claude Code paint on screen while it streams a markdown table?

Throwaway named herdr session, torn down. Samples the visible grid at 0.3 s across one turn
whose answer is a markdown table, and writes every distinct frame.
"""
import json, os, shutil, subprocess, sys, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-mdtable-%d" % os.getpid()
SOCK = os.path.expanduser("~/.config/herdr/sessions/%s/herdr.sock" % NAME)
OUT = os.path.join(SP, "frames")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM", "CLAUDE_CODE_CHILD_SESSION")}


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


PROMPT = ("Without using any tools, answer with a markdown table and nothing else: a table of "
          "six terminal control sequences with columns Sequence, Name, and a third column What it does "
          "holding a full sentence of at least fifteen words each so the rows are long. "
          "Put one short sentence of prose before the table.")


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(OUT)
    start()
    try:
        work = os.path.join(SP, "work")
        os.makedirs(work, exist_ok=True)
        ws = call("workspace.create", {"label": "mdtable", "cwd": work})
        pane = ws["root_pane"]["pane_id"]
        print("pane", pane, "session", NAME, flush=True)
        lay = call("pane.layout", {"pane_id": pane})
        print("layout", json.dumps(lay)[:400], flush=True)
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": pane, "text": "claude\r"})
        time.sleep(9.0)
        call("pane.send_text", {"pane_id": pane, "text": PROMPT})
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "\r"})

        last, n, t0 = None, 0, time.time()
        while time.time() - t0 < 150:
            s = screen(pane)
            if s != last:
                last = s
                n += 1
                with open(os.path.join(OUT, "f%03d.txt" % n), "w") as f:
                    f.write(s)
            time.sleep(0.3)
        print("wrote %d frames" % n, flush=True)
    finally:
        stop()


if __name__ == "__main__":
    main()
