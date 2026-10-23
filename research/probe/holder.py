"""A stand-in for `kampr-hold` and `kampr attach` (docs/17), enough to test the lifecycle against
a real herdr.

    python3 holder.py serve SOCK [--pidfile PATH] -- argv...   detaches, owns argv on its own pty
    kampr holder.py attach SOCK                                 a client; run it through a `kampr` symlink

The holder keeps every byte (uncapped: a probe runs for seconds) and replays them to each new
client, then streams live. A client sends `I<len><bytes>` for input and `R<rows><cols>` for its
size; the last client to send a size owns it.
"""

import fcntl
import os
import pty
import select
import signal
import socket
import struct
import sys
import termios
import tty


def serve(path, argv, pidfile):
    if os.fork():
        return
    os.setsid()
    pid, master = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    if pidfile:
        with open(pidfile, "w") as f:
            f.write(f"{os.getpid()} {pid}\n")
    devnull = os.open(os.devnull, os.O_RDWR)
    for fd in (0, 1):
        os.dup2(devnull, fd)
    os.dup2(os.open(os.environ.get("HOLDER_LOG", os.devnull), os.O_WRONLY | os.O_CREAT | os.O_APPEND), 2)
    log = bytearray()
    lsock = socket.socket(socket.AF_UNIX)
    if os.path.exists(path):
        os.unlink(path)
    lsock.bind(path)
    lsock.listen()
    clients = {}
    while True:
        ready, _, _ = select.select([master, lsock, *clients], [], [])
        if master in ready:
            try:
                data = os.read(master, 65536)
            except OSError:
                data = b""
            if not data:
                break
            log += data
            for c in list(clients):
                try:
                    c.sendall(data)
                except OSError:
                    clients.pop(c)
                    c.close()
        if lsock in ready:
            c, _ = lsock.accept()
            c.sendall(bytes(log))
            clients[c] = b""
        for c in [c for c in ready if c in clients]:
            try:
                data = c.recv(65536)
            except OSError:
                data = b""
            if not data:
                clients.pop(c)
                c.close()
                continue
            buf = clients[c] + data
            while buf:
                if buf[:1] == b"R" and len(buf) >= 5:
                    rows, cols = struct.unpack("HH", buf[1:5])
                    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
                    buf = buf[5:]
                elif buf[:1] == b"I" and len(buf) >= 5:
                    (n,) = struct.unpack("I", buf[1:5])
                    if len(buf) < 5 + n:
                        break
                    os.write(master, buf[5 : 5 + n])
                    buf = buf[5 + n :]
                else:
                    break
            clients[c] = buf
    os.unlink(path)
    os._exit(0)


def attach(path):
    c = socket.socket(socket.AF_UNIX)
    c.connect(path)

    def resize(*_):
        rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8))
        c.sendall(b"R" + struct.pack("HH", rows, cols))

    resize()
    signal.signal(signal.SIGWINCH, resize)
    if os.isatty(0):
        tty.setraw(0, termios.TCSANOW)
    while True:
        ready, _, _ = select.select([c, 0], [], [])
        if c in ready:
            data = c.recv(65536)
            if not data:
                break
            os.write(1, data)
        if 0 in ready:
            data = os.read(0, 65536)
            if not data:
                break
            c.sendall(b"I" + struct.pack("I", len(data)) + data)


if sys.argv[1] == "serve":
    rest = sys.argv[3:]
    pidfile = None
    if rest[0] == "--pidfile":
        pidfile, rest = rest[1], rest[2:]
    serve(sys.argv[2], rest[1:], pidfile)
else:
    attach(sys.argv[2])
