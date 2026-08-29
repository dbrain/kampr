#!/usr/bin/env python3
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
SOCK = os.path.join(os.path.expanduser("~/.config"), "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin")
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
    o = {"t": time.time()}
    g = rpc("pane.get", {"pane_id": pane}, sock_path=SOCK)["result"]["pane"]
    o["agent"], o["status"] = g.get("agent"), g.get("agent_status")
    pi = rpc("pane.process_info", {"pane_id": pane}, sock_path=SOCK)["result"]["process_info"]
    o["fg"] = [p["name"] for p in pi.get("foreground_processes", [])]
    return o

def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/bin/bash", os.path.join(BIN, "claude"))
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    start()
    try:
        for label, cmd in [
            ("bash -c 'sleep 6'  (single simple command)", "%s/claude -c '%s/sleep 6'" % (BIN, BIN)),
            ("bash -c 'sleep 6; :' (two commands)", "%s/claude -c '%s/sleep 6; :'" % (BIN, BIN)),
        ]:
            pane = call("workspace.create", {"label": "e", "cwd": SP})["root_pane"]["pane_id"]
            time.sleep(1.5)
            call("pane.send_text", {"pane_id": pane, "text": cmd + "\r"})
            print("\n== %s ==" % label, flush=True)
            t0 = time.time(); prev = None
            while time.time() - t0 < 8.0:
                s = snap(pane); k = (s["agent"], s["status"], tuple(s["fg"]))
                if k != prev:
                    print("  %5.2f agent=%-8r status=%-9r fg=%s" % (time.time() - t0, s["agent"], s["status"], s["fg"]), flush=True)
                    prev = k
                time.sleep(0.05)
            call("pane.close", {"pane_id": pane})
            time.sleep(0.5)
    finally:
        stop()

main()
