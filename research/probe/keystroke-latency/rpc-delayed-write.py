"""Does a herdr call stall because the request lost a race with the server's first read?

Predicts: writing the request *after* a pause the server cannot have missed makes the stall
happen every time, at the same value. Writing it into the same instant as the connect makes it
rare. Same call either way.
"""
import json, os, socket, sys, time

SOCK = os.environ["HERDR_SOCKET_PATH"]
N = int(os.environ.get("N", "40"))


def run(pause):
    took = []
    for _ in range(N):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(20)
        s.connect(SOCK)
        if pause:
            time.sleep(pause)
        t = time.perf_counter()
        s.sendall((json.dumps({"id": "p", "method": "pane.list", "params": {}}) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            c = s.recv(65536)
            if not c:
                break
            buf += c
        took.append((time.perf_counter() - t) * 1000)
        s.close()
        time.sleep(0.05)
    took.sort()
    def q(p): return took[round((len(took) - 1) * p)]
    return (f"pause {pause*1000:6.1f} ms before writing: n={len(took)} min {took[0]:.2f}  "
            f"p50 {q(.5):.2f}  p90 {q(.9):.2f}  max {took[-1]:.2f} ms  "
            f"over-50ms {sum(1 for v in took if v > 50)}")


for pause in [float(x) for x in os.environ.get('PAUSES', '0,0.002,0.01,0.05,0.2,0.5').split(',')]:
    print(run(pause))
