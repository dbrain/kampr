#!/usr/bin/env python3
"""Probe #406: what the live pending test's own screen looks like at a chosen shell prompt width.

    python3 pending-echo-width.py '["ci$ ", "runner@runnervmgx7h7:/tmp$ "]' [--blank]

Creates a throwaway headless session, types the `printf` the test types, and dumps the visible
rows with their lengths. `--blank` uses the fixed form, whose format string opens with `\n`.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import time

name = "kampr-probe-echowidth-" + str(os.getpid())
home = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.environ["HOME"], ".config")
sockdir = os.path.join(home, "herdr", "sessions", name)
sock = os.path.join(sockdir, "herdr.sock")


def rpc(method, params=None, timeout=25):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock)
    s.sendall((json.dumps({"id": "p", "method": method, "params": params or {}}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    return json.loads(buf.decode().splitlines()[0])


prompts = json.loads(sys.argv[1])
lead = "\\n" if "--blank" in sys.argv[2:] else ""
command = "printf '%sDo you want to make this edit?\\n\\n 1. Yes\\n 2. No\\n'\n" % lead

subprocess.Popen(
    ["herdr", "server", "--session", name], stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
)
for _ in range(200):
    if os.path.exists(sock):
        break
    time.sleep(0.1)
time.sleep(0.5)
try:
    rpc("workspace.create", {"label": "probe", "cwd": "/tmp"})
    time.sleep(1.0)
    found = []

    def walk(node):
        if isinstance(node, dict):
            if "pane_id" in node:
                found.append(node["pane_id"])
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    walk(rpc("session.snapshot", {}))
    pane = found[0]
    for prompt in prompts:
        rpc("pane.send_text", {"pane_id": pane, "text": "PS1=" + json.dumps(prompt) + "\n"})
        time.sleep(0.8)
        rpc("pane.send_text", {"pane_id": pane, "text": "clear\n"})
        time.sleep(0.8)
        rpc("pane.send_text", {"pane_id": pane, "text": command})
        time.sleep(1.5)
        read = rpc("pane.read", {"pane_id": pane, "source": "visible",
                                 "format": "text", "strip_ansi": True})
        text = read.get("result", read)["read"]["text"]
        print(f"=== PS1 is {len(prompt)} columns")
        for i, row in enumerate(text.rstrip("\n").split("\n")):
            print(f"{i:3d} |{row}| {len(row)}")
finally:
    try:
        rpc("server.stop", {}, timeout=5)
    except OSError:
        pass
    time.sleep(1.0)
    shutil.rmtree(sockdir, ignore_errors=True)
