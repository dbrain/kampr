"""One write or two: does splitting the request body from its terminating newline cost 100 ms?

`kampr_herdr::Herdr::call` writes the JSON, then writes `b"\\n"`, then flushes. If herdr's first
look at the connection catches the body without the newline, the newline is not seen for a whole
poll interval.
"""
import json, os, socket, time

SOCK = os.environ["HERDR_SOCKET_PATH"]
N = int(os.environ.get("N", "200"))
BODY = json.dumps({"id": "p", "method": "pane.list", "params": {}}).encode()


def run(split, gap_us=0):
    took = []
    for _ in range(N):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(20)
        s.connect(SOCK)
        t = time.perf_counter()
        if split:
            s.sendall(BODY)
            if gap_us:
                time.sleep(gap_us / 1e6)
            s.sendall(b"\n")
        else:
            s.sendall(BODY + b"\n")
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
    label = f"{'two writes' if split else 'one write '}{f' +{gap_us}us' if gap_us else '       '}"
    return (f"{label}: n={len(took)} min {took[0]:.2f}  p50 {q(.5):.2f}  p90 {q(.9):.2f}  "
            f"max {took[-1]:.2f} ms  over-50ms {sum(1 for v in took if v > 50)}")


print(run(False))
print(run(True))
print(run(True, 200))
print(run(False))
