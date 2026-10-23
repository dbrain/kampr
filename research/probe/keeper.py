#!/usr/bin/env python3
"""How small the piece that must never restart can be (docs/17 §1): a keeper owning one PTY master
and its child, upgraded by re-exec and killed outright, with a child counting as fast as it can.

    python3 research/probe/keeper.py probe

The keeper runs as a `systemd-run --user` unit of type `notify`, appends everything the master
yields to a log on tmpfs, hands the master to systemd's fd store, re-execs itself on SIGUSR1 with
the master left open, and recovers the master from LISTEN_FDS when systemd restarts it. The child
counts `SEQ n` lines without pause and records any SIGHUP, so a gap or a hangup shows in the log.
"""

import array
import os
import pty
import signal
import socket
import subprocess
import sys
import tempfile
import time

COUNTER = 'trap "echo HUP >> $KEEPER_DIR/hup; exit 1" HUP; i=0; while :; do echo "SEQ $i"; i=$((i+1)); done'


def notify(msg, fds=()):
    addr = os.environ.get("NOTIFY_SOCKET")
    if not addr:
        return
    if addr.startswith("@"):
        addr = "\0" + addr[1:]
    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as s:
        anc = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", fds))] if fds else []
        s.sendmsg([msg.encode()], anc, 0, addr)


def serve(d):
    os.environ["KEEPER_DIR"] = d
    if "KEEPER_FD" in os.environ:
        master, how = int(os.environ.pop("KEEPER_FD")), "re-exec"
    elif os.environ.get("LISTEN_PID") == str(os.getpid()):
        master, how = 3, "fd store"
    else:
        pid, master = pty.fork()
        if pid == 0:
            os.execvp("bash", ["bash", "--norc", "-c", COUNTER])
        with open(os.path.join(d, "child"), "w") as f:
            f.write(str(pid))
        if os.environ.get("KEEPER_STORE") == "1":
            notify("FDSTORE=1\nFDNAME=pty", [master])
        how = "spawn"
    with open(os.path.join(d, "events"), "a") as f:
        f.write(f"{time.time():.3f} {os.getpid()} {how}\n")
    notify("READY=1")
    upgrade = []
    signal.signal(signal.SIGUSR1, lambda *_: upgrade.append(1))
    with open(os.path.join(d, "log"), "ab", buffering=0) as log:
        while True:
            if upgrade:
                os.set_inheritable(master, True)
                os.environ["KEEPER_FD"] = str(master)
                os.execv(sys.executable, [sys.executable, "-I", os.path.abspath(__file__), "serve", d])
            try:
                data = os.read(master, 65536)
            except InterruptedError:
                continue
            except OSError:
                return
            if not data:
                return
            log.write(data)


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().split()[2] != "Z"
    except OSError:
        return False


def ppid(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            p = f.read().rsplit(")", 1)[1].split()[1]
        with open(f"/proc/{p}/comm") as f:
            return f"{p} {f.read().strip()}"
    except OSError:
        return None


def check(d, since):
    with open(os.path.join(d, "log"), "rb") as f:
        f.seek(since)
        data = f.read()
    seqs = [int(l[4:]) for l in data.split(b"\r\n")[1:-1] if l.startswith(b"SEQ ") and l[4:].isdigit()]
    gaps = [(a, b) for a, b in zip(seqs, seqs[1:]) if b != a + 1]
    lost = sum(b - a - 1 for a, b in gaps if b > a)
    return f"{len(seqs)} lines, gaps {len(gaps)}, {lost} lines lost (~{lost * len(f'SEQ {seqs[-1]}') + lost * 2} bytes), last {seqs[-1] if seqs else None}"


def probe():
    for store in (True, False):
        d = tempfile.mkdtemp(prefix="kk-", dir=os.environ.get("XDG_RUNTIME_DIR"))
        unit = f"kph-keeper-{int(store)}-{os.getpid()}"
        props = ["Type=notify", "NotifyAccess=all", "Restart=always", "RestartSec=3", "KillMode=process",
                 f"FileDescriptorStoreMax={8 if store else 0}"]
        show = lambda: subprocess.run(["systemctl", "--user", "show", unit, "-p", "MainPID,NRestarts,NFileDescriptorStore"],
                                      capture_output=True, text=True).stdout.split()
        try:
            subprocess.run(["systemd-run", "--user", "-q", "--unit", unit, *sum((["-p", p] for p in props), []),
                            f"--setenv=KEEPER_STORE={int(store)}", sys.executable, "-I", os.path.abspath(__file__), "serve", d],
                           check=True)
            time.sleep(1)
            child = int(open(os.path.join(d, "child")).read())
            keeper = int(show()[0].split("=")[1])
            print(f"=== fd store {'on' if store else 'off'}: keeper {keeper}, child {child}, {show()}")
            for step in ("re-exec", "SIGKILL"):
                mark = os.path.getsize(os.path.join(d, "log"))
                os.kill(keeper, signal.SIGUSR1 if step == "re-exec" else signal.SIGKILL)
                time.sleep(5)
                after = show()
                print(f"  {step}: {after} | child alive {alive(child)} (parent {ppid(child)}) | hup {os.path.exists(os.path.join(d, 'hup'))} | {check(d, mark)}")
                keeper = int(after[0].split("=")[1])
                if not keeper:
                    break
            print("  events:", open(os.path.join(d, "events")).read().strip().replace("\n", " | "))
        finally:
            subprocess.run(["systemctl", "--user", "stop", unit], stderr=subprocess.DEVNULL)
            subprocess.run(["systemctl", "--user", "reset-failed", unit], stderr=subprocess.DEVNULL)
            try:
                os.kill(int(open(os.path.join(d, "child")).read()), signal.SIGKILL)
            except (OSError, ValueError):
                pass
            subprocess.run(["rm", "-rf", d])


if sys.argv[1] == "serve":
    serve(sys.argv[2])
else:
    probe()
