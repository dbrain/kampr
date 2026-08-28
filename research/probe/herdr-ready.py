#!/usr/bin/env python3
"""Probe #326: when a spawned herdr is listed, when its socket accepts, and when it answers.

Takes a throwaway config root, spawns a default server into it, and watches all three at 2 ms.
"""
import json
import os
import socket
import subprocess
import sys
import time

root = sys.argv[1]
env = dict(os.environ, XDG_CONFIG_HOME=root, HERDR_CONFIG_PATH=root + "/herdr/config.toml")
env.pop("HERDR_SOCKET_PATH", None)
env.pop("HERDR_SESSION", None)
sock = root + "/herdr/herdr.sock"

started = time.monotonic()
subprocess.Popen(
    ["herdr", "server"], env=env, stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
)
listed = connected = answered = None
while time.monotonic() - started < 30:
    if listed is None:
        out = subprocess.run(["herdr", "session", "list", "--json"], env=env, capture_output=True, text=True)
        if '"running":true' in out.stdout:
            listed = (time.monotonic() - started) * 1000
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(2)
        client.connect(sock)
        if connected is None:
            connected = (time.monotonic() - started) * 1000
        client.sendall(json.dumps({"id": "p", "method": "session.snapshot", "params": {}}).encode() + b"\n")
        line = client.makefile().readline()
        client.close()
        if line.strip() and "result" in json.loads(line):
            answered = (time.monotonic() - started) * 1000
            break
    except OSError:
        pass
    time.sleep(0.002)

ms = lambda v: "never" if v is None else f"{round(v)}ms"
print(f"listed running {ms(listed)}  connect {ms(connected)}  first RPC result {ms(answered)}")
