import os, pty, sys, time, json, select, tty, termios

CHILD = r'''
import os, sys, time
out = open(sys.argv[1], "w")
start = time.monotonic()
while True:
    try:
        b = os.read(0, 4096)
    except OSError:
        break
    if not b:
        break
    out.write(repr(b) + "\n"); out.flush()
    if b'q' in b:
        break
'''

def run(gap_ms, log):
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(sys.executable, [sys.executable, "-c", CHILD, log])
    # raw-ish: turn off echo and canonical so the child sees bytes as typed
    attrs = termios.tcgetattr(fd)
    tty.setraw(fd)
    time.sleep(0.3)
    if gap_ms is None:
        os.write(fd, b"\x1b\r")
    else:
        os.write(fd, b"\x1b")
        if gap_ms:
            time.sleep(gap_ms / 1000.0)
        os.write(fd, b"\r")
    time.sleep(0.3)
    os.write(fd, b"q")
    time.sleep(0.2)
    os.close(fd)
    os.waitpid(pid, 0)
    return open(log).read().strip().splitlines()

results = {}
for label, gap in [("one write", None), ("two writes, no gap", 0), ("two writes, 1ms", 1), ("two writes, 3ms", 3)]:
    reads = []
    for trial in range(5):
        log = "/tmp/claude-1000/-home-dbrain-dev-kampr/bb6aff3d-39b2-41f6-bddf-ac7b83fb4335/scratchpad/child.log"
        open(log, "w").close()
        reads.append(run(gap, log))
    results[label] = reads
print(json.dumps(results, indent=1))
