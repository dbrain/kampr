import os, signal, subprocess, sys, time

CTRL = sys.argv[1]
LOG = sys.argv[2]
SLEEPBIN = sys.argv[3]

signal.signal(signal.SIGTTOU, signal.SIG_IGN)
signal.signal(signal.SIGTTIN, signal.SIG_IGN)

tty = os.open("/dev/tty", os.O_RDWR)
try:
    os.setpgid(0, 0)
except OSError:
    pass
try:
    os.tcsetpgrp(tty, os.getpgrp())
except OSError:
    pass

def note(s):
    with open(LOG, "a") as f:
        f.write("%.4f %s\n" % (time.time(), s))

def title(t):
    sys.stdout.write("\033]0;" + t + "\007")
    sys.stdout.flush()

note("agent pid=%d pgrp=%d" % (os.getpid(), os.getpgrp()))
title(sys.argv[4])
note("title idle")

while True:
    with open(CTRL) as f:
        line = f.read().strip()
    if not line:
        continue
    parts = line.split()
    cmd = parts[0]
    if cmd == "quit":
        note("quit")
        break
    elif cmd == "title":
        title(" ".join(parts[1:]).replace("BRAILLE", "⠋").replace("SPARK", "✳").replace("HALF", "◐"))
        note("title -> %r" % (" ".join(parts[1:]),))
    elif cmd in ("same", "own"):
        dur = float(parts[1])
        note("spawn %s %.2f BEGIN" % (cmd, dur))
        if cmd == "same":
            p = subprocess.Popen([SLEEPBIN, str(dur)])
            note("child pid=%d pgrp=%d" % (p.pid, os.getpgid(p.pid)))
            p.wait()
        else:
            def pre():
                os.setpgid(0, 0)
                try:
                    os.tcsetpgrp(tty, os.getpgrp())
                except OSError:
                    pass
            p = subprocess.Popen([SLEEPBIN, str(dur)], preexec_fn=pre)
            note("child pid=%d pgrp=%d" % (p.pid, os.getpgid(p.pid)))
            p.wait()
            try:
                os.tcsetpgrp(tty, os.getpgrp())
            except OSError as e:
                note("tcsetpgrp back failed %s" % e)
        note("spawn %s %.2f END" % (cmd, dur))
    elif cmd == "idlefor":
        time.sleep(float(parts[1]))
        note("idlefor done")
