import os
import pty
import select
import sys
import time
import json

import pyte

COLS, ROWS = 160, 50
screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)
fifo, shot = sys.argv[1], sys.argv[2]
cmd = sys.argv[3:]

pid, fd = pty.fork()
if pid == 0:
    os.environ["COLUMNS"], os.environ["LINES"] = str(COLS), str(ROWS)
    os.execvp(cmd[0], cmd)

import fcntl, struct, termios
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
if not os.path.exists(fifo):
    os.mkfifo(fifo)
inp = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
last = 0.0
prev = ''
while True:
    r, _, _ = select.select([fd, inp], [], [], 0.05)
    if fd in r:
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if not data:
            break
        stream.feed(data)
    if inp in r:
        data = os.read(inp, 65536)
        if data:
            os.write(fd, data.replace(b"\\r", b"\r").replace(b"\\e", b"\x1b"))
        else:
            os.close(inp)
            inp = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
    if time.time() - last > 0.05:
        text = "\n".join(line.rstrip() for line in screen.display).rstrip() + "\n"
        if text != prev:
            with open(shot, "w") as f:
                f.write(text)
            with open(shot + ".log", "a") as f:
                f.write(json.dumps({"t": round(time.time(), 3), "screen": text}) + "\n")
            prev = text
        last = time.time()
