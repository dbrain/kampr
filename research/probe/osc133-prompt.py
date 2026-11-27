#!/usr/bin/env python3
"""Whether the shells on this machine mark their prompt with OSC 133 A in a plain pty, which is what
ends a program's `working`/`blocked` OSC 7501 records in a native session (docs/24).

    python3 -I research/probe/osc133-prompt.py [shell ...]    # default: zsh bash fish

Each shell runs interactive with the operator's own startup files (read, never written), in a pty
of this script's own with TERM=xterm-256color, for 4 s, then gets `true` and 2 s more. Prints the
OSC numbers it wrote and whether `133;A` was among them.
"""

import os
import pty
import re
import select
import shutil
import signal
import sys
import time

OSC = re.compile(rb"\x1b\](\d+);([^\x07\x1b]{0,12})")


def run(shell):
    path = shutil.which(shell)
    if not path:
        return f"{shell}: not installed"
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        for k in [k for k in os.environ if k.startswith(("HERDR_", "KAMPR_", "TMUX", "KITTY_", "WEZTERM", "TERM_PROGRAM", "VTE_", "GHOSTTY"))]:
            del os.environ[k]
        os.execv(path, [shell, "-i"])
    out = b""
    end = time.time() + 4
    sent = False
    while time.time() < end or not sent:
        if time.time() >= end and not sent:
            os.write(fd, b"true\r")
            sent, end = True, time.time() + 2
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                out += os.read(fd, 65536)
            except OSError:
                break
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    codes = sorted({(m.group(1).decode(), m.group(2).decode("latin1")[:4]) for m in OSC.finditer(out)})
    return f"{shell} ({path}): OSC {codes}; 133;A {'yes' if b'\x1b]133;A' in out else 'no'}"


for shell in sys.argv[1:] or ["zsh", "bash", "fish"]:
    print(run(shell))
