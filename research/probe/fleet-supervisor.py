#!/usr/bin/env python3
"""Probe #334-#337: what a supervisor that OWNS the pty can see that an outside watcher cannot.

#331 measured the outside view and it is mostly blind: `/proc/<pid>/syscall` is EPERM under yama
`ptrace_scope=1` for anything the node did not fork, and a root job under `sudo` denies `wchan`
and `fd/0` as well. This asks the other question: if the thing running the command is the command's
own parent, and shares its privilege, does the same read succeed?

Run it twice — once as the operator, once under `sudo` — and compare.
"""

import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time

X86_READ = 0


def proc(pid, name):
    try:
        with open(f"/proc/{pid}/{name}") as fh:
            return fh.read().strip()
    except PermissionError as exc:
        return f"<EPERM {exc.errno}>"
    except OSError as exc:
        return f"<errno {exc.errno}>"


def blocked_in_read(pid):
    """The confident signal: the job is parked in read(2) on fd 0."""
    raw = proc(pid, "syscall")
    if raw.startswith("<") or raw == "running":
        return None, raw
    parts = raw.split()
    try:
        nr, fd = int(parts[0]), int(parts[1], 16)
    except (ValueError, IndexError):
        return None, raw
    return (nr == X86_READ and fd == 0), raw


def echo_of(fd):
    try:
        return "ECHO on" if termios.tcgetattr(fd)[3] & termios.ECHO else "ECHO OFF"
    except Exception as exc:  # noqa: BLE001
        return f"<{exc}>"


def run(argv, label, settle=2.0, answer=None):
    """Spawn argv on a pty this process owns, let it settle, then look at it from the parent."""
    master, slave = pty.openpty()
    child = subprocess.Popen(
        argv,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        preexec_fn=os.setsid,
        close_fds=True,
    )
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

    # The job is the child, or the child's own child when it re-execs (sudo does).
    targets = [child.pid]
    kids = proc(child.pid, "task/%d/children" % child.pid)
    if not kids.startswith("<"):
        targets += [int(p) for p in kids.split()]

    print(f"\n--- {label}   (parent uid={os.getuid()})")
    print(f"    pty ECHO now: {echo_of(master)}")
    for pid in targets:
        verdict, raw = blocked_in_read(pid)
        mark = {True: "WAITING", False: "busy", None: "unknown"}[verdict]
        print(
            f"    pid {pid:>8} {proc(pid, 'comm'):<10} state={proc(pid, 'stat').split(') ')[-1].split()[0] if not proc(pid, 'stat').startswith('<') else '?'}"
            f"  wchan={proc(pid, 'wchan'):<20} syscall={raw!r:<34} -> {mark}"
        )
    printable = out.decode("utf-8", "replace")
    lines = [l for l in printable.replace("\r", "").split("\n") if l.strip()]
    print(f"    last line on the pty: {lines[-1] if lines else '(nothing written)'!r}")

    if answer is not None:
        os.write(master, answer)
        time.sleep(1.2)
        try:
            more = os.read(master, 65536).decode("utf-8", "replace")
            print(f"    after answering {answer!r}: {more.replace(chr(13), '')[:160]!r}")
        except OSError:
            pass

    try:
        os.killpg(os.getpgid(child.pid), signal.SIGKILL)
    except OSError:
        pass
    child.wait(timeout=5)
    os.close(master)


def exit_code_case():
    """The other half: does the supervisor get the real exit code, without scraping for it?"""
    print("\n--- exit codes, straight from wait(2)")
    for argv, expect in ((["true"], 0), (["false"], 1), (["sh", "-c", "exit 42"], 42)):
        master, slave = pty.openpty()
        child = subprocess.Popen(argv, stdin=slave, stdout=slave, stderr=slave)
        os.close(slave)
        code = child.wait(timeout=10)
        os.close(master)
        print(f"    {' '.join(argv):<20} -> {code}  (expected {expect})  {'ok' if code == expect else 'MISMATCH'}")


def main():
    print(f"uid={os.getuid()}  ptrace_scope={open('/proc/sys/kernel/yama/ptrace_scope').read().strip()}")
    run(["cat"], "cat — parked in read() on the pty")
    run(["sleep", "5"], "sleep 5 — SILENT WORK, must not read as waiting")
    run(["python3", "-c", "while True: pass"], "busy loop — CPU work")
    run(["python3", "-c", "input('Continue? [Y/n] ')"], "a bare prompt with no shell around it")
    run(
        ["python3", "-c", "import getpass; getpass.getpass('Password: ')"],
        "a password prompt — does ECHO go off on a pty with no ble.sh on it?",
    )
    run(["sudo", "pacman", "-S", "bash"], "pacman -S bash — the real one", settle=4.0, answer=b"n\n")
    exit_code_case()


if __name__ == "__main__":
    main()
