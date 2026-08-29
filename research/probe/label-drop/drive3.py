#!/usr/bin/env python3
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
SOCK = os.path.join(os.path.expanduser("~/.config"), "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin"); CTRL = os.path.join(SP, "c3.fifo"); ALOG = os.path.join(SP, "a3.log")
CLEAN_ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and k != "TERM_PROGRAM"}
WORKING = "◐ probe working"

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
    o = {}
    g = rpc("pane.get", {"pane_id": pane}, sock_path=SOCK)["result"]["pane"]
    o["agent"], o["status"], o["title"] = g.get("agent"), g.get("agent_status"), g.get("terminal_title")
    try:
        ex = rpc("agent.explain", {"target": pane}, sock_path=SOCK)["result"]["explain"]
        o["fb"] = ex.get("fallback_reason"); mr = ex.get("matched_rule")
        o["rule"] = mr.get("id") if isinstance(mr, dict) else mr
        o["exstate"] = ex.get("state")
        for r in ex.get("evaluated_rules", []):
            if r.get("region") == "osc_title":
                o["osc"] = r["evidence"].get("region_preview"); o["oscb"] = r["evidence"].get("region_bytes"); break
    except Exception as e: o["ex"] = str(e)
    return o

def ctrl(m):
    with open(CTRL, "w") as f: f.write(m + "\n")

def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/usr/bin/python3", os.path.join(BIN, "claude"))
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    for p in (CTRL, ALOG):
        if os.path.exists(p): os.remove(p)
    os.mkfifo(CTRL)
    start()
    try:
        pane = call("workspace.create", {"label": "d3", "cwd": SP})["root_pane"]["pane_id"]
        time.sleep(2.0)
        call("pane.send_text", {"pane_id": pane, "text": "%s/claude %s/fake_agent2.py %s %s %s/sleep '%s'\r" % (BIN, SP, CTRL, ALOG, BIN, WORKING)})
        for t in (3, 6, 10, 20, 40):
            while time.time() % 1: break
            time.sleep(0)
        for wait in (3, 3, 4, 10, 20):
            time.sleep(wait)
            print("t+%2ds  %s" % (wait, json.dumps(snap(pane), ensure_ascii=False)), flush=True)
        print("--- now rewrite the BYTE-IDENTICAL title ---", flush=True)
        ctrl("title HALF probe working")
        for wait in (0.5, 0.5, 1.0, 2.0):
            time.sleep(wait)
            print("   +%.1fs %s" % (wait, json.dumps(snap(pane), ensure_ascii=False)), flush=True)
        ctrl("quit"); time.sleep(1)
    finally:
        stop()

main()
