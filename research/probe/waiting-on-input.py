#!/usr/bin/env python3
"""Probe #331-#333: can a non-descendant process tell that a pane's job is waiting for input?

Kampr's node is not an ancestor of anything herdr spawned, and this machine runs yama
`ptrace_scope=1`, so every /proc file that needs PTRACE_MODE_ATTACH is a question rather than an
assumption. Runs against its own throwaway `herdr server` (started by the caller, dir in
`probe331.dir`); the operator's own server never takes part.
"""

import json
import os
import sys
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc  # noqa: E402

SCRATCH = "/tmp/claude-1000/-home-dbrain-dev-kampr/9749f4c0-eb82-46f0-a7b8-ac6a015b523c/scratchpad"
DIR = open(f"{SCRATCH}/probe331.dir").read().strip()
SOCK = f"{DIR}/herdr/sessions/fleetprobe/herdr.sock"


def call(method, params=None):
    reply = rpc(method, params or {}, sock_path=SOCK)
    if reply is None:
        raise SystemExit(f"no reply to {method}")
    if "error" in reply and reply["error"]:
        raise SystemExit(f"{method}: {reply['error']}")
    return reply.get("result", reply)


def read_text(pid, name):
    try:
        with open(f"/proc/{pid}/{name}") as fh:
            return fh.read().strip()
    except PermissionError as exc:
        return f"<EPERM {exc.errno}>"
    except OSError as exc:
        return f"<errno {exc.errno}>"


def stat_state(pid):
    try:
        raw = open(f"/proc/{pid}/stat").read()
    except OSError as exc:
        return f"<errno {exc.errno}>"
    return raw[raw.rindex(")") + 2 :].split()[0]


def fd0(pid):
    try:
        return os.readlink(f"/proc/{pid}/fd/0")
    except OSError as exc:
        return f"<errno {exc.errno}>"


def comm(pid):
    return read_text(pid, "comm")


def children(pid):
    out = []
    try:
        for thread in os.listdir(f"/proc/{pid}/task"):
            try:
                out += open(f"/proc/{pid}/task/{thread}/children").read().split()
            except OSError:
                pass
    except OSError:
        pass
    return [int(p) for p in dict.fromkeys(out)]


def walk(shell, depth=0):
    """Every process below the shell, nearest first — procfs.rs's walk, in miniature."""
    found = []
    for child in children(shell):
        if comm(child).startswith("<"):
            continue
        found.append((depth, child))
        if depth < 3:
            found += walk(child, depth + 1)
    return found


def echo_bit(pts):
    """termios belongs to the tty, not to the fd, so opening it read-only reads the pane's own."""
    try:
        fh = os.open(pts, os.O_RDONLY | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as exc:
        return f"<errno {exc.errno}>"
    try:
        return "ECHO on" if termios.tcgetattr(fh)[3] & termios.ECHO else "ECHO OFF"
    except Exception as exc:  # noqa: BLE001
        return f"<{exc}>"
    finally:
        os.close(fh)


def sample(shell, label):
    procs = walk(shell)
    rows = []
    for _, pid in procs:
        pts = fd0(pid)
        rows.append(
            {
                "pid": pid,
                "comm": comm(pid),
                "state": stat_state(pid),
                "wchan": read_text(pid, "wchan"),
                "syscall": read_text(pid, "syscall"),
                "fd0": pts,
                "echo": echo_bit(pts) if pts.startswith("/dev/pts/") else "-",
            }
        )
    print(f"\n--- {label}")
    if not rows:
        print("  (no job below the shell)")
        shell_pts = fd0(shell)
        print(
            f"  shell {shell} comm={comm(shell)} state={stat_state(shell)} "
            f"wchan={read_text(shell, 'wchan')} syscall={read_text(shell, 'syscall')} "
            f"fd0={shell_pts} {echo_bit(shell_pts) if shell_pts.startswith('/dev/pts/') else ''}"
        )
    for row in rows:
        print(
            f"  {row['pid']:>8} {row['comm']:<12} state={row['state']:<3} "
            f"wchan={row['wchan']:<24} syscall={row['syscall']}"
        )
        print(f"           fd0={row['fd0']}  {row['echo']}")
    return rows


def screen(pane):
    reply = call(
        "pane.read",
        {"pane_id": pane, "source": "visible", "format": "text", "strip_ansi": True},
    )
    return reply["read"]["text"]


def tail(pane, n=6):
    lines = [l for l in screen(pane).splitlines() if l.strip()]
    return lines[-n:]


def main():
    snap = call("session.snapshot")["snapshot"]
    if not snap["panes"]:
        call("workspace.create", {"label": "fleetprobe", "cwd": os.path.expanduser("~"), "focus": True})
        time.sleep(1.5)
        snap = call("session.snapshot")["snapshot"]
    pane = snap["panes"][0]["pane_id"]
    info = call("pane.process_info", {"pane_id": pane})["process_info"]
    shell = info["shell_pid"]
    print(f"pane={pane} shell_pid={shell} ppid-chain-of-me={os.getpid()}")
    print(f"herdr's own view: {json.dumps(info)}")
    print(f"am I an ancestor of the shell? {'yes' if is_ancestor(os.getpid(), shell) else 'NO'}")

    def send(text, settle=1.6):
        call("pane.send_text", {"pane_id": pane, "text": text + "\n"})
        time.sleep(settle)

    send("stty -echo 2>/dev/null; stty echo; clear", 1.0)
    sample(shell, "A. shell sitting at its prompt (no job)")

    send("cat", 1.4)
    sample(shell, "B. `cat` — blocked reading the tty")
    call("pane.send_keys", {"pane_id": pane, "keys": ["ctrl+c"]})
    time.sleep(0.8)

    send('read -p "Continue? [Y/n] " reply', 1.4)
    rows = sample(shell, "C. bash `read -p` — the generic prompt shape")
    print(f"  screen tail: {tail(pane, 2)!r}")
    call("pane.send_keys", {"pane_id": pane, "keys": ["ctrl+c"]})
    time.sleep(0.8)

    send("sleep 8", 1.4)
    sample(shell, "D. `sleep 8` — SILENT WORK, must not look like waiting")
    call("pane.send_keys", {"pane_id": pane, "keys": ["ctrl+c"]})
    time.sleep(0.8)

    send("python3 -c 'while True: pass'", 1.4)
    sample(shell, "E. busy loop — CPU work")
    call("pane.send_keys", {"pane_id": pane, "keys": ["ctrl+c"]})
    time.sleep(0.8)

    send("clear; sudo -k -p 'kampr-probe password: ' true", 1.8)
    sample(shell, "F. sudo password prompt — is ECHO readable from outside?")
    print(f"  screen tail: {tail(pane, 2)!r}")
    call("pane.send_keys", {"pane_id": pane, "keys": ["ctrl+c"]})
    time.sleep(0.8)

    send("clear; sudo pacman -S bash", 3.5)
    rows = sample(shell, "G. `pacman -S bash` — the real prompt")
    print("  screen tail:")
    for line in tail(pane, 8):
        print(f"    {line!r}")
    call("pane.send_text", {"pane_id": pane, "text": "n\n"})
    time.sleep(1.5)
    print("  after answering n:")
    for line in tail(pane, 4):
        print(f"    {line!r}")
    return rows


def is_ancestor(me, pid):
    seen = 0
    while pid > 1 and seen < 40:
        try:
            raw = open(f"/proc/{pid}/stat").read()
        except OSError:
            return False
        pid = int(raw[raw.rindex(")") + 2 :].split()[1])
        if pid == me:
            return True
        seen += 1
    return False


if __name__ == "__main__":
    main()
