#!/usr/bin/env python3
"""Does the osc_title detection region survive a label release + re-acquire?
Also: does SIGSTOP-ing the agent (or a herdr detection restart) lose it?"""
import json, os, shutil, signal, subprocess, sys, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
SOCK = os.path.join(os.path.expanduser("~/.config"), "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin")
CLEAN_ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and k != "TERM_PROGRAM"}
WORKING = "◐ probe working"

def call(m, p=None, quiet=False):
    r = rpc(m, p or {}, sock_path=SOCK)
    if not r or "error" in r:
        if quiet: return {"__err": r.get("error") if r else None}
        raise SystemExit("%s: %s" % (m, r))
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
        for r in ex.get("evaluated_rules", []):
            if r.get("region") == "osc_title":
                o["oscb"] = r["evidence"].get("region_bytes"); break
    except Exception as e: o["ex"] = str(e)[:60]
    return o

def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/usr/bin/python3", os.path.join(BIN, "claude"))
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    fifo = os.path.join(SP, "f7.fifo")
    if os.path.exists(fifo): os.remove(fifo)
    os.mkfifo(fifo)
    start()
    try:
        pane = call("workspace.create", {"label": "d7", "cwd": SP})["root_pane"]["pane_id"]
        time.sleep(1.5)
        call("pane.send_text", {"pane_id": pane, "text": "%s/claude %s/fake_agent3.py %s %s/x.log %s/sleep '%s' 3\r"
                                % (BIN, SP, fifo, SP, BIN, WORKING)})
        time.sleep(9)
        print("before      : %s" % json.dumps(snap(pane), ensure_ascii=False), flush=True)
        r = call("pane.release_agent", {"pane_id": pane, "source": "kampr-probe", "agent": "claude"}, quiet=True)
        print("release_agent -> %s" % json.dumps(r)[:160], flush=True)
        for w in (0.4, 0.4, 1.0, 2.0, 4.0, 8.0):
            time.sleep(w)
            print("  +%.1f %s" % (w, json.dumps(snap(pane), ensure_ascii=False)), flush=True)
        print("clear_agent_authority -> %s" % json.dumps(call("pane.clear_agent_authority", {"pane_id": pane, "source": "kampr-probe"}, quiet=True))[:120], flush=True)
        for w in (0.4, 1.0, 2.0, 4.0, 6.0):
            time.sleep(w)
            print("  +%.1f %s" % (w, json.dumps(snap(pane), ensure_ascii=False)), flush=True)
        with open(fifo, "w") as f: f.write("quit\n")
        time.sleep(1)
    finally:
        stop()
        os.path.exists(fifo) and os.remove(fifo)

main()
