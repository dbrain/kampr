#!/usr/bin/env python3
"""Whether anything herdr gives a client says a pane has bracketed paste (DECSET 2004) on: the live
`observe` stream, `pane.read format:ansi` and `pane.get`, with a pane's program toggling 2004; and what
the terminal of `pane.process_info`'s `shell_pid` reads as, its line discipline read at a shell prompt and in
`read -s`, from a throwaway herdr session torn down afterwards.

    python3 research/probe/observe-bracketed-mode.py
"""

import base64
import json
import os
import shutil
import subprocess
import sys
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc

name = f"kampr-probe-2004-{os.getpid()}"
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
    time.sleep(3)
    rpc("pane.send_text", {"pane_id": pane, "text": "sh\r"}, sock_path=sock)
    time.sleep(1)
    rpc("pane.send_text", {"pane_id": pane, "text": "printf '\\033[?2004hON\\n'; sleep 2; printf '\\033[?2004lOFF\\n'; sleep 2\r"}, sock_path=sock)
    time.sleep(1)
    get_on = json.dumps(rpc("pane.get", {"pane_id": pane}, sock_path=sock))
    read_on = rpc("pane.read", {"pane_id": pane, "source": "visible", "format": "ansi", "lines": 10}, sock_path=sock)["result"]["read"]["text"]
    time.sleep(3)
    get_off = json.dumps(rpc("pane.get", {"pane_id": pane}, sock_path=sock))
    observe.terminate()
    stream = b""
    for line in observe.stdout.read().splitlines():
        try:
            stream += base64.b64decode(json.loads(line)["bytes"])
        except (ValueError, KeyError):
            pass
    print("observe bytes:", len(stream), "saw ON:", b"ON" in stream, "saw OFF:", b"OFF" in stream)
    print("observe ?2004h:", stream.count(b"\x1b[?2004h"), "?2004l:", stream.count(b"\x1b[?2004l"))
    print("observe private modes:", sorted(set(m.decode() for m in __import__('re').findall(rb"\x1b\[\?[0-9;]+[hl]", stream))))
    print("pane.read ansi while on carries the escape:", "\x1b[?2004" in read_on)
    print("pane.get differs between on and off:", get_on != get_off, "mentions bracket/paste:", "paste" in get_on or "bracket" in get_on)
    def lflag():
        info = rpc("pane.process_info", {"pane_id": pane}, sock_path=sock)["result"]
        tty = os.readlink(f"/proc/{info['process_info']['shell_pid']}/fd/0")
        fd = os.open(tty, os.O_RDONLY | os.O_NOCTTY | os.O_NONBLOCK)
        try:
            flags = termios.tcgetattr(fd)[3]
        finally:
            os.close(fd)
        return tty, bool(flags & termios.ICANON), bool(flags & termios.ECHO)
    pane = rpc("workspace.create", {"focus": False}, sock_path=sock)["result"]["root_pane"]["pane_id"]
    time.sleep(5)
    print("login shell prompt (the operator's own rc): tty, ICANON, ECHO =", lflag())
    rpc("pane.send_text", {"pane_id": pane, "text": "read -rs secret\r"}, sock_path=sock)
    time.sleep(1)
    print("login shell read -s: tty, ICANON, ECHO =", lflag())
    rpc("pane.send_text", {"pane_id": pane, "text": "x\r"}, sock_path=sock)
    time.sleep(1)
    rpc("pane.send_text", {"pane_id": pane, "text": "cat\r"}, sock_path=sock)
    time.sleep(1)
    print("cat: tty, ICANON, ECHO =", lflag())
    rpc("pane.send_text", {"pane_id": pane, "text": "\x04"}, sock_path=sock)
    time.sleep(1)
    rpc("pane.send_text", {"pane_id": pane, "text": "exec bash --norc --noprofile\r"}, sock_path=sock)
    time.sleep(1.5)
    print("bash prompt: tty, ICANON, ECHO =", lflag())
    rpc("pane.send_text", {"pane_id": pane, "text": "read -rs secret\r"}, sock_path=sock)
    time.sleep(1)
    print("read -s: tty, ICANON, ECHO =", lflag())
    rpc("pane.send_text", {"pane_id": pane, "text": "x\r"}, sock_path=sock)
    time.sleep(0.5)
    rpc("pane.send_text", {"pane_id": pane, "text": "exec zsh -f\r"}, sock_path=sock)
    time.sleep(1.5)
    print("zsh -f prompt: tty, ICANON, ECHO =", lflag())
finally:
    try:
        rpc("server.stop", {}, sock_path=sock, timeout=3)
    except OSError:
        pass
    server.wait(timeout=10)
    shutil.rmtree(os.path.dirname(sock), ignore_errors=True)
