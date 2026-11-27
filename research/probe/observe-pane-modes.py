#!/usr/bin/env python3
"""Whether anything herdr gives a client says what DEC private modes a pane's program set (mouse
tracking 1000/1002/1003, SGR 1006, alternate scroll 1007, application cursor 1, alternate screen
1049/47/1047): the live `observe` stream with claude-like, muffit-like, less and vim output in a
throwaway herdr session torn down afterwards, `pane.get`/`pane.list` with the program live, and the
host's own pane-graphics reading. Run it with HERDR on PATH.

    python3 research/probe/observe-pane-modes.py
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

name = f"kampr-probe-modes-{os.getpid()}"
sock = os.path.expanduser(f"~/.config/herdr/sessions/{name}/herdr.sock")
env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "PI_"))}
server = subprocess.Popen(["herdr", "server", "--session", name], stdin=subprocess.DEVNULL,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
CASES = [
    ("claude-like", "printf '\\033[?1049h\\033[?1000h\\033[?1006h\\033[?1004h'; sleep 2; printf '\\033[?1000l\\033[?1006l\\033[?1049l'"),
    ("muffit-like", "printf '\\033[?1049h\\033[?1000h\\033[?1002h\\033[?1003h\\033[?1006h'; sleep 2; printf '\\033[?1003l\\033[?1002l\\033[?1000l\\033[?1006l\\033[?1049l'"),
    ("less", "seq 1 200 > /tmp/kampr-probe-modes.txt; less /tmp/kampr-probe-modes.txt"),
    ("vim", "vim -u NONE -c 'set mouse=a' -c 'set ttymouse=sgr' /tmp/kampr-probe-modes.txt"),
]
MODE = re.compile(rb"\x1b\[\?[0-9;]+[hl]")
try:
    for _ in range(100):
        if os.path.exists(sock):
            break
        time.sleep(0.1)
    time.sleep(0.5)
    print("herdr", subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip())
    for label, cmd in CASES:
        pane = rpc("workspace.create", {"focus": False}, sock_path=sock)["result"]["root_pane"]["pane_id"]
        observe = subprocess.Popen(["herdr", "terminal", "session", "observe", pane, "--cols", "100", "--rows", "30"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                   env={**env, "HERDR_SOCKET_PATH": sock})
        time.sleep(3)
        rpc("pane.send_text", {"pane_id": pane, "text": "sh\r"}, sock_path=sock)
        time.sleep(1)
        rpc("pane.send_text", {"pane_id": pane, "text": cmd + "\r"}, sock_path=sock)
        time.sleep(1)
        live = json.dumps([rpc("pane.get", {"pane_id": pane}, sock_path=sock),
                           rpc("pane.list", {}, sock_path=sock)])
        read = rpc("pane.read", {"pane_id": pane, "source": "visible", "format": "ansi", "lines": 30}, sock_path=sock)["result"]["read"]["text"]
        time.sleep(3)
        if label in ("less", "vim"):
            rpc("pane.send_text", {"pane_id": pane, "text": "q" if label == "less" else "\x1b:q!\r"}, sock_path=sock)
            time.sleep(1)
        observe.terminate()
        stream = b""
        for line in observe.stdout.read().splitlines():
            try:
                stream += base64.b64decode(json.loads(line)["bytes"])
            except (ValueError, KeyError):
                pass
        print(label, "| observe bytes:", len(stream), "| private modes in stream:",
              sorted(set(m.decode().replace("\x1b", "ESC") for m in MODE.findall(stream))),
              "| ansi read carries a private mode:", bool(MODE.search(read.encode())),
              "| pane.get/list mention mouse/alt/mode:", bool(re.search(r"mouse|alt_?screen|app_?cursor|decckm", live, re.I)))
finally:
    try:
        rpc("server.stop", {}, sock_path=sock, timeout=3)
    except OSError:
        pass
    server.wait(timeout=10)
    shutil.rmtree(os.path.dirname(sock), ignore_errors=True)
    try:
        os.remove("/tmp/kampr-probe-modes.txt")
    except OSError:
        pass
