#!/usr/bin/env python3
"""Probe #407: the rows a real agent puts above its own permission dialog.

    python3 dialog-rows.py <scratch dir> [binary] [prompt]

Runs the harness in a throwaway headless session, sends one prompt that needs approval, and dumps
the visible screen once a numbered menu is on it. What the row directly above the question is, is
the whole point: `pending::question_above` joins upward through anything that is not blank, not a
rule and not sentence-final.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import time

name = "kampr-probe-dialogrows-" + str(os.getpid())
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


cwd = sys.argv[1]
launch = sys.argv[2] if len(sys.argv) > 2 else "claude --permission-mode default"
ask = sys.argv[3] if len(sys.argv) > 3 else (
    "Run exactly this bash command and nothing else: curl -s https://example.com | head -3"
)
# A session marker in the environment turns transcript saving off and changes the chrome.
env = {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE")}

subprocess.Popen(
    ["herdr", "server", "--session", name], env=env, stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
)
for _ in range(200):
    if os.path.exists(sock):
        break
    time.sleep(0.1)
time.sleep(0.5)
try:
    rpc("workspace.create", {"label": "probe", "cwd": cwd})
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
    rpc("pane.send_text", {"pane_id": pane, "text": launch + "\n"})
    time.sleep(12.0)
    rpc("pane.send_text", {"pane_id": pane, "text": ask})
    time.sleep(1.0)
    rpc("pane.send_keys", {"pane_id": pane, "keys": ["Enter"]})
    text = ""
    for _ in range(14):
        time.sleep(5.0)
        read = rpc("pane.read", {"pane_id": pane, "source": "visible",
                                 "format": "text", "strip_ansi": True})
        text = read.get("result", read)["read"]["text"]
        if "1. Yes" in text:
            break
    for i, row in enumerate(text.rstrip("\n").split("\n")):
        print(f"{i:3d} |{row}|")
finally:
    try:
        rpc("server.stop", {}, timeout=5)
    except OSError:
        pass
    time.sleep(1.0)
    shutil.rmtree(sockdir, ignore_errors=True)
