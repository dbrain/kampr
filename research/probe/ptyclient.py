import os, pty, fcntl, termios, struct, sys, time, select, subprocess
def spawn(cmd, cols, rows, env=None):
    pid, fd = pty.fork()
    if pid == 0:
        e = dict(os.environ); e.update(env or {}); e["TERM"]="xterm-256color"
        os.execvpe(cmd[0], cmd, e)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    return pid, fd
def drain(fd, secs):
    out=b""; t0=time.time()
    while time.time()-t0 < secs:
        r,_,_ = select.select([fd],[],[],0.2)
        if r:
            try: c=os.read(fd, 65536)
            except OSError: break
            if not c: break
            out+=c
    return out
