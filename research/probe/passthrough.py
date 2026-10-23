"""A stand-in for `kampr attach` (docs/17): runs argv on a pty of its own, sized to its own tty,
and copies bytes both ways verbatim.

    kampr passthrough.py [--comm NAME] [--pidfile PATH] -- argv...

Run it through a symlink named `kampr` to python3, so the pane's foreground process is called
`kampr` the way the real attach client would be. `--comm` renames it with PR_SET_NAME afterwards.
"""

import ctypes
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import tty


def size(fd):
    return struct.unpack("HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\0" * 8))


def main():
    args = sys.argv[1:]
    comm = pidfile = None
    while args and args[0] != "--":
        flag, value, args = args[0], args[1], args[2:]
        if flag == "--comm":
            comm = value
        elif flag == "--pidfile":
            pidfile = value
    argv = args[1:]
    if comm:
        ctypes.CDLL(None).prctl(15, comm.encode(), 0, 0, 0)
    pid, master = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    if pidfile:
        with open(pidfile, "w") as f:
            f.write(f"{os.getpid()} {pid}\n")
    winsize = lambda *_: fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", *size(0)))
    winsize()
    signal.signal(signal.SIGWINCH, winsize)
    if os.isatty(0):
        tty.setraw(0, termios.TCSANOW)
    while True:
        try:
            ready, _, _ = select.select([master, 0], [], [])
        except InterruptedError:
            continue
        if master in ready:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            os.write(1, data)
        if 0 in ready:
            data = os.read(0, 65536)
            if not data:
                break
            os.write(master, data)
    _, status = os.waitpid(pid, 0)
    sys.exit(os.waitstatus_to_exitcode(status))


main()
