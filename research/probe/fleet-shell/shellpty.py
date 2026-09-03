#!/usr/bin/env python3
"""Does putting a shell on a fleet pty reintroduce #333, and what does it do to the tree?

#337 measured that ECHO going off is an honest password signal on a pty with no shell on it, and
#333 measured that ble.sh leaves an *interactive* shell's tty with ECHO already off. A fleet run
that understands `&&` needs a shell on that pty. This asks whether a NON-interactive, NON-login
`bash -c` carries the confound with it, and what the process tree looks like underneath it.

The pty is set up exactly as `kampr_fleet::exec::Supervisor::spawn` sets it up: setsid **and**
TIOCSCTTY, so the child has a controlling terminal. That second half matters — without it
`getpass` cannot open `/dev/tty` and silently falls back to fd 0, which is a different measurement
from the one the product makes.
"""

import fcntl, os, pty, select, signal, subprocess, sys, termios, time

READ_FD0 = {0, 17, 19, 275, 295, 327}  # x86_64 read-family, first arg is an fd


def proc(pid, name):
    try:
        with open(f"/proc/{pid}/{name}") as fh:
            return fh.read().strip()
    except PermissionError as e:
        return f"<EPERM {e.errno}>"
    except OSError as e:
        return f"<errno {e.errno}>"


def fd_target(pid, fd):
    try:
        return os.readlink(f"/proc/{pid}/fd/{fd}")
    except OSError as e:
        return f"<errno {e.errno}>"


def waiting(pid):
    raw = proc(pid, "syscall")
    if raw.startswith("<") or raw in ("running", ""):
        return "unknown", raw, None
    f = raw.split()
    try:
        nr, a0 = int(f[0]), int(f[1], 16)
    except (ValueError, IndexError):
        return "unknown", raw, None
    if nr < 0:
        return "unknown", raw, None
    if nr in READ_FD0 and a0 < 16:
        return "WAITING(fd %d)" % a0, raw, fd_target(pid, a0)
    return "busy", raw, None


def tree(pid, depth=0):
    out = [(pid, depth)]
    if depth >= 4:
        return out
    kids = proc(pid, f"task/{pid}/children")
    if not kids.startswith("<"):
        for k in kids.split():
            out += tree(int(k), depth + 1)
    return out


def modes(fd):
    try:
        lf = termios.tcgetattr(fd)[3]
        return ("ECHO on" if lf & termios.ECHO else "ECHO OFF",
                "ICANON on" if lf & termios.ICANON else "ICANON OFF")
    except Exception as e:
        return (f"<{e}>", "")


def case(label, argv, settle=2.0, interactive_ctty=True):
    master, slave = pty.openpty()
    pts = os.ttyname(slave)

    def setup():
        os.setsid()
        if interactive_ctty:
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    child = subprocess.Popen(argv, stdin=slave, stdout=slave, stderr=slave,
                             preexec_fn=setup, close_fds=True, env=env)
    os.close(slave)
    out = b""
    deadline = time.time() + settle
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.1)
        if r:
            try:
                out += os.read(master, 65536)
            except OSError:
                break
    e, c = modes(master)
    print(f"--- {label}")
    print(f"    argv    : {argv}")
    print(f"    pty     : {pts}")
    print(f"    termios : {e}, {c}")
    print(f"    output  : {out[-120:]!r}")
    for pid, depth in tree(child.pid):
        state, raw, target = waiting(pid)
        comm = proc(pid, "comm")
        on = f"  -> {target}" if target else ""
        print(f"    {'  '*depth}pid {pid} [{comm}] {state}{on}   syscall={raw[:40]}")
    print()
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except OSError:
        pass
    child.wait()
    os.close(master)


GETPASS = 'python3 -c "import getpass; getpass.getpass()"'
BASH = "/usr/bin/bash"

if __name__ == "__main__":
    print("== (a) the password signal, with and without a shell in front of it")
    case("no shell, getpass (the #337 baseline)",
         ["python3", "-c", "import getpass; getpass.getpass()"])
    case("bash -c getpass", [BASH, "-c", GETPASS])
    case("bash -c 'read -s'", [BASH, "-c", 'read -s -p "Password: " x'])
    case("bash -c 'true && getpass'", [BASH, "-c", f"true && {GETPASS}"])
    case("bash -c 'echo hi | getpass'", [BASH, "-c", f"echo hi | {GETPASS}"])
    case("no shell, su - (the #339 case)", ["su", "-"], settle=3.0)
    case("bash -c 'su -'", [BASH, "-c", "su -"], settle=3.0)
    print("== the confound itself: an INTERACTIVE shell on the same pty (#333)")
    case("bash -i (idle at its own prompt)", [BASH, "-i"], settle=3.0)
    case("bash -lic (idle at its own prompt)", [BASH, "-lic", "sleep 30"], settle=4.0)

    print("== (b) what the tree looks like, and whose fd 0 is the pty")
    case("bash -c 'cat'", [BASH, "-c", "cat"])
    case("bash -c 'sleep 30'", [BASH, "-c", "sleep 30"])
    case("bash -c 'true && cat'", [BASH, "-c", "true && cat"])
    case("bash -c 'false && cat' (chain that stops)", [BASH, "-c", "false && cat; sleep 30"])
    case("bash -c 'cat | cat' (front of the pipe holds the pty)", [BASH, "-c", "cat | cat"])
    case("bash -c 'sleep 30 | cat' (NOTHING holds the pty)", [BASH, "-c", "sleep 30 | cat"])
    case("bash -c 'sleep 30 & cat'", [BASH, "-c", "sleep 30 & cat"])
    case("bash -c 'sleep 30 & wait' (a background job, nothing asking)",
         [BASH, "-c", "sleep 30 & wait"])
    print("== does a non-interactive bash -c load ble.sh?")
    case("bash -c 'echo BLE=$BLE_VERSION FLAGS=$-; cat'",
         [BASH, "-c", 'echo "BLE=${BLE_VERSION-none} FLAGS=$-"; cat'])
