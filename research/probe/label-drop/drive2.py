#!/usr/bin/env python3
import json, os, shutil, subprocess, sys, threading, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc

SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
SOCK = os.path.join(os.path.expanduser("~/.config"), "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin"); CTRL = os.path.join(SP, "ctrl2.fifo"); ALOG = os.path.join(SP, "agent2.log")
CLEAN_ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and k != "TERM_PROGRAM"}

def call(m, p=None):
    r = rpc(m, p or {}, sock_path=SOCK)
    if not r or "error" in r: raise SystemExit("%s: %s" % (m, r))
    return r["result"]

def start():
    subprocess.Popen(["herdr", "server", "--session", NAME], env=CLEAN_ENV,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(200):
        if os.path.exists(SOCK): time.sleep(0.8); return
        time.sleep(0.1)
    raise SystemExit("no server")

def stop():
    try: rpc("server.stop", {}, sock_path=SOCK)
    except Exception: pass
    for _ in range(80):
        if not os.path.exists(SOCK): break
        time.sleep(0.1)
    shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)

def snap(pane):
    out = {"t": time.time()}
    try:
        g = rpc("pane.get", {"pane_id": pane}, sock_path=SOCK)["result"]["pane"]
        out["agent"], out["status"], out["title"] = g.get("agent"), g.get("agent_status"), g.get("terminal_title")
    except Exception as e: out["err"] = str(e)
    try:
        pi = rpc("pane.process_info", {"pane_id": pane}, sock_path=SOCK)["result"]["process_info"]
        out["fg"] = [p["name"] for p in pi.get("foreground_processes", [])]
    except Exception as e: out["fg"] = ["ERR"]
    try:
        ex = rpc("agent.explain", {"target": pane}, sock_path=SOCK)["result"]["explain"]
        out["fb"] = ex.get("fallback_reason")
        mr = ex.get("matched_rule")
        out["rule"] = mr.get("id") if isinstance(mr, dict) else mr
        out["exstate"] = ex.get("state")
        for r in ex.get("evaluated_rules", []):
            if r.get("region") == "osc_title":
                out["osc"] = r["evidence"].get("region_preview"); out["oscb"] = r["evidence"].get("region_bytes"); break
            
    except Exception as e: out["ex_err"] = str(e)
    return out

class S(threading.Thread):
    def __init__(s, pane, per=0.08):
        super().__init__(daemon=True); s.pane, s.per, s.rows, s.go = pane, per, [], True
    def run(s):
        while s.go:
            t = time.time(); s.rows.append(snap(s.pane))
            d = s.per - (time.time() - t)
            if d > 0: time.sleep(d)
    def stop(s): s.go = False; s.join(timeout=3)

def ctrl(m):
    with open(CTRL, "w") as f: f.write(m + "\n")

def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/usr/bin/python3", os.path.join(BIN, "claude"))
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    for p in (CTRL, ALOG):
        if os.path.exists(p): os.remove(p)
    os.mkfifo(CTRL)
    start(); marks = []
    def mark(x): marks.append((time.time(), x)); print("[mark] %s" % x, flush=True)
    try:
        ws = call("workspace.create", {"label": "drop2", "cwd": SP})
        pane = ws["root_pane"]["pane_id"]; print("pane", pane, flush=True)
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "%s/claude %s/fake_agent.py %s %s %s/sleep\r" % (BIN, SP, CTRL, ALOG, BIN)})
        time.sleep(5.0)
        s = S(pane); s.start(); time.sleep(2.0)
        print("baseline:", json.dumps(snap(pane)), flush=True)

        def phase(label, msg, wait):
            mark("BEGIN " + label)
            if msg: ctrl(msg)
            time.sleep(wait)
            mark("END " + label)
            print("   %-28s -> %s" % (label, json.dumps(snap(pane))), flush=True)

        phase("rewrite idle title", "title SPARK probe pane", 3)
        phase("write working title", "title HALF probe pane", 3)
        phase("rewrite same working", "title HALF probe pane", 3)
        phase("child same 5s", "same 5.0", 9)
        phase("child own 5s", "own 5.0", 9)
        phase("child own 12s", "own 12.0", 17)
        phase("write working title again", "title HALF probe pane", 3)
        phase("child own 20s", "own 20.0", 26)
        mark("BEGIN quit")
        ctrl("quit"); time.sleep(6); mark("END quit")
        print("   after quit -> %s" % json.dumps(snap(pane)), flush=True)
        s.stop()
        json.dump({"rows": s.rows, "marks": marks}, open(os.path.join(SP, "samples2.json"), "w"))
        print("rows", len(s.rows), flush=True)
    finally:
        stop()

main()
