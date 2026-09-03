"""Does a busy herdr look at a new connection later, and so miss more requests?

The node is not a lone caller: it holds an `observe` child and an `events.subscribe` stream per
watched pane and sweeps on top. This asks whether other traffic on the socket widens the window
the request has to land in.
"""
import json, os, socket, threading, time

SOCK = os.environ["HERDR_SOCKET_PATH"]
N = int(os.environ.get("N", "200"))
REQ = (json.dumps({"id": "p", "method": "pane.list", "params": {}}) + "\n").encode()
stop = threading.Event()


def one():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(20)
    t = time.perf_counter()
    s.connect(SOCK)
    s.sendall(REQ)
    buf = b""
    while b"\n" not in buf:
        c = s.recv(65536)
        if not c:
            break
        buf += c
    s.close()
    return (time.perf_counter() - t) * 1000


def hammer():
    while not stop.is_set():
        try:
            one()
        except OSError:
            pass


def run(label, background):
    stop.clear()
    threads = [threading.Thread(target=hammer, daemon=True) for _ in range(background)]
    for t in threads:
        t.start()
    took = [one() or time.sleep(0.05) for _ in range(N)]
    took = [v for v in took if v]
    stop.set()
    for t in threads:
        t.join(2)
    took.sort()
    def q(p): return took[round((len(took) - 1) * p)]
    slow = sum(1 for v in took if v > 50)
    print(f"{label}: n={len(took)} min {took[0]:.2f}  p50 {q(.5):.2f}  p90 {q(.9):.2f}  "
          f"max {took[-1]:.2f} ms  over-50ms {slow} ({100*slow/len(took):.0f}%)")


run("herdr otherwise idle      ", 0)
run("one other caller hammering", 1)
run("four other callers        ", 4)
run("herdr otherwise idle      ", 0)
