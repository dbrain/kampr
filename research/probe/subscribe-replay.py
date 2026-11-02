"""Does `events.subscribe` replay anything when it opens, and does a status change made before the
subscription is live ever reach it?

    python3 research/probe/subscribe-replay.py

One throwaway named session, stopped and removed on the way out. The subscription list is the
node's own: every topology kind plus `pane.agent_status_changed` for the pane under test.
"""

import json
import os
import shutil
import socket
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc
from importlib import import_module

arming = import_module("done-arming")

KINDS = [
    "layout.updated", "pane.created", "pane.closed", "pane.updated", "pane.exited", "pane.moved",
    "pane.agent_detected", "tab.created", "tab.closed", "tab.renamed", "tab.moved",
    "workspace.created", "workspace.updated", "workspace.closed", "workspace.renamed",
    "workspace.moved", "workspace.reordered", "worktree.created", "worktree.opened",
    "worktree.removed",
]
PANE = "w2:p1"


def subscribe(sock):
    subs = [{"type": k} for k in KINDS] + [{"type": "pane.agent_status_changed", "pane_id": PANE}]
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sock)
    s.sendall((json.dumps({"id": "probe", "method": "events.subscribe",
                           "params": {"subscriptions": subs}}) + "\n").encode())
    s.settimeout(0.05)
    return s


def drain(s, seconds):
    lines, buf, until = [], b"", time.time() + seconds
    while time.time() < until:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            continue
        if not chunk:
            break
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            lines.append(json.loads(line))
    return lines


def kinds(lines):
    return [l.get("event") or ("ack" if "result" in l or "error" in l else "?") for l in lines]


def report(sock, state):
    rpc("pane.report_agent", {"pane_id": PANE, "agent": "claude", "source": "kampr-probe",
                              "state": state}, sock_path=sock)


def labelled(sock, tmp):
    binary = os.path.join(tmp, "claude")
    shutil.copy(shutil.which("sleep"), binary)
    rpc("workspace.create", {"label": "kampr", "cwd": "/tmp"}, sock_path=sock)
    rpc("workspace.create", {"label": "probe", "cwd": tmp}, sock_path=sock)
    time.sleep(0.4)
    rpc("pane.send_text", {"pane_id": PANE, "text": f"{binary} 600\n"}, sock_path=sock)
    for _ in range(200):
        pane = rpc("pane.get", {"pane_id": PANE}, sock_path=sock)["result"]["pane"]
        if pane.get("agent") == "claude" and pane["agent_status"] != "unknown":
            return pane
        time.sleep(0.05)
    raise SystemExit("herdr never labelled the pane")


def main():
    name = f"kampr-probe-subreplay-{os.getpid()}"
    sock = arming.start(name)
    tmp = tempfile.mkdtemp()
    try:
        version = os.popen("herdr --version").read().strip()
        print(version, labelled(sock, tmp)["agent_status"])
        for run in range(5):
            report(sock, "idle")
            time.sleep(0.3)
            report(sock, "blocked")
            s = subscribe(sock)
            opened = kinds(drain(s, 3.0))
            report(sock, "working")
            after = kinds(drain(s, 1.0))
            s.close()
            print(f"run {run}: reported blocked before subscribing -> on open {opened}; "
                  f"reported working after -> {after}")
    finally:
        arming.stop(name)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
