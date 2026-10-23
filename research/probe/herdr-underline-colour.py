#!/usr/bin/env python3
"""Which of SGR 58 (underline colour) and SGR 5 (blink) herdr passes on, on its live stream and on
`pane.read format:ansi`, from a throwaway herdr session torn down afterwards.

    python3 research/probe/herdr-underline-colour.py
"""

import base64
import json
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc

LINE = "printf '\\033[4;58;2;255;0;0mUL\\033[59mPLAIN\\033[0m \\033[4:3;58:5:196mCURL\\033[0m \\033[5mBLINK\\033[0m\\n'"
SGR = re.compile(r"\x1b\[[0-9;:]*m")

name = f"kampr-probe-ul-{os.getpid()}"
sock = os.path.expanduser(f"~/.config/herdr/sessions/{name}/herdr.sock")
env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "PI_"))}
server = subprocess.Popen(["herdr", "server", "--session", name], stdin=subprocess.DEVNULL,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
try:
    for _ in range(100):
        if os.path.exists(sock):
            break
        time.sleep(0.1)
    time.sleep(0.5)
    pane = rpc("workspace.create", {"focus": False}, sock_path=sock)["result"]["root_pane"]["pane_id"]
    observe = subprocess.Popen(["herdr", "terminal", "session", "observe", pane, "--cols", "80", "--rows", "10"],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                               env={**env, "HERDR_SOCKET_PATH": sock})
    time.sleep(4)
    rpc("pane.send_text", {"pane_id": pane, "text": "clear; " + LINE + "\r"}, sock_path=sock)
    time.sleep(3)
    observe.terminate()
    stream = b""
    for line in observe.stdout.read().splitlines():
        try:
            stream += base64.b64decode(json.loads(line)["bytes"])
        except (ValueError, KeyError):
            pass
    at = stream.rfind(b"UL")
    print("observe, around the text:", repr(stream[max(0, at - 40):at + 90]))
    print("observe pens:", sorted(set(SGR.findall(stream.decode("utf8", "replace")))))
    read = rpc("pane.read", {"pane_id": pane, "source": "visible", "format": "ansi", "lines": 5}, sock_path=sock)
    print("pane.read ansi pens:", sorted(set(SGR.findall(read["result"]["read"]["text"]))))
finally:
    try:
        rpc("server.stop", {}, sock_path=sock, timeout=3)
    except OSError:
        pass
    server.wait(timeout=10)
    shutil.rmtree(os.path.dirname(sock), ignore_errors=True)
