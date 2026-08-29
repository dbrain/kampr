#!/usr/bin/env python3
"""Does a child holding a herdr pane's foreground drop the pane's agent label?

Throwaway named herdr session, torn down. Nothing touches the operator's default server.
"""
import json, os, shutil, subprocess, sys, threading, time

sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
CFG = os.path.expanduser("~/.config")
SOCK = os.path.join(CFG, "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin")
CTRL = os.path.join(SP, "ctrl.fifo")
ALOG = os.path.join(SP, "agent.log")

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if k not in ("HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                          "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH",
                          "HERDR_BIN_PATH", "TERM_PROGRAM")}


def call(method, params=None, sock=None):
    r = rpc(method, params or {}, sock_path=sock or SOCK)
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
    for _ in range(80):
        if not os.path.exists(SOCK):
            break
        time.sleep(0.1)
    shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)


class Sampler(threading.Thread):
    def __init__(self, pane, period=0.04):
        super().__init__(daemon=True)
        self.pane, self.period, self.rows, self.go = pane, period, [], True

    def run(self):
        while self.go:
            t = time.time()
            try:
                g = rpc("pane.get", {"pane_id": self.pane}, sock_path=SOCK)["result"]["pane"]
            except Exception as e:
                g = {"err": str(e)}
            try:
                pi = rpc("pane.process_info", {"pane_id": self.pane},
                         sock_path=SOCK)["result"]["process_info"]
                fg = [p["name"] for p in pi.get("foreground_processes", [])]
                pgid = pi.get("foreground_process_group_id")
            except Exception as e:
                fg, pgid = ["ERR:" + str(e)], None
            self.rows.append({"t": t, "agent": g.get("agent"), "status": g.get("agent_status"),
                              "title": g.get("terminal_title"), "fg": fg, "pgid": pgid})
            d = self.period - (time.time() - t)
            if d > 0:
                time.sleep(d)

    def stop(self):
        self.go = False
        self.join(timeout=3)


def note(marks, s):
    marks.append((time.time(), s))
    print("  [mark] %.3f %s" % (time.time(), s), flush=True)


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def ctrl(msg):
    with open(CTRL, "w") as f:
        f.write(msg + "\n")


def explain(pane):
    try:
        return rpc("agent.explain", {"target": pane}, sock_path=SOCK)["result"]["explain"]
    except Exception as e:
        return {"error": str(e)}


def osc_preview(ex):
    for r in ex.get("evaluated_rules", []):
        if r.get("region") == "osc_title":
            return r["evidence"].get("region_preview"), r["evidence"].get("region_bytes")
    return None, None


def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/usr/bin/python3", os.path.join(BIN, "claude"))
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    for p in (CTRL, ALOG):
        if os.path.exists(p):
            os.remove(p)
    os.mkfifo(CTRL)

    start()
    marks = []
    try:
        ws = call("workspace.create", {"label": "drop", "cwd": SP})
        pane = ws["root_pane"]["pane_id"]
        print("pane", pane, "session", NAME, flush=True)
        time.sleep(2.0)
        send(pane, "%s/claude %s/fake_agent.py %s %s %s/sleep\r"
             % (BIN, SP, CTRL, ALOG, BIN))
        time.sleep(4.0)
        s = Sampler(pane)
        s.start()
        time.sleep(2.0)
        g = rpc("pane.get", {"pane_id": pane}, sock_path=SOCK)["result"]["pane"]
        print("baseline:", {k: g.get(k) for k in ("agent", "agent_status", "terminal_title")}, flush=True)
        if g.get("agent") != "claude":
            pi = rpc("pane.process_info", {"pane_id": pane}, sock_path=SOCK)["result"]
            print("NO LABEL. process_info:", json.dumps(pi), flush=True)
            s.stop()
            return

        plan = []
        for mode in ("same", "own"):
            for dur in (0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 10.0):
                plan.append((mode, dur))

        for mode, dur in plan:
            note(marks, "BEGIN %s %.1f" % (mode, dur))
            ctrl("%s %s" % (mode, dur))
            time.sleep(dur + 6.0)
            note(marks, "END %s %.1f" % (mode, dur))
            ex = explain(pane)
            pv, nb = osc_preview(ex)
            print("    after %s %.1f: agent=%r status=%r fallback=%r matched=%r osc_preview=%r (%s bytes)"
                  % (mode, dur, ex.get("agent"), ex.get("state") or ex.get("status"),
                     ex.get("fallback_reason"), (ex.get("matched_rule") or {}),
                     pv, nb), flush=True)
            time.sleep(2.0)

        ctrl("quit")
        time.sleep(1.5)
        s.stop()
        with open(os.path.join(SP, "samples.json"), "w") as f:
            json.dump({"rows": s.rows, "marks": marks}, f)
        print("wrote samples.json (%d rows)" % len(s.rows), flush=True)
    finally:
        stop()


if __name__ == "__main__":
    main()
