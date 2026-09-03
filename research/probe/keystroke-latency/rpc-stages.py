"""Where a herdr socket call spends its time: connect, write, first byte of the reply.

Every Kampr call dials fresh — herdr closes the connection after one response — so a stall in
the accept path is a stall on every keystroke.
"""
import json, os, socket, sys, time

SOCK = os.environ["HERDR_SOCKET_PATH"]
METHOD = os.environ.get("METHOD", "pane.list")
PARAMS = json.loads(os.environ.get("PARAMS", "{}"))
N = int(os.environ.get("N", "300"))
GAP = float(os.environ.get("GAP", "0.05"))


def quantiles(s):
    s = sorted(s)
    def q(p): return s[round((len(s) - 1) * p)]
    return (f"n={len(s)} min {s[0]:.2f}  p50 {q(.5):.2f}  p90 {q(.9):.2f}  p99 {q(.99):.2f}  "
            f"max {s[-1]:.2f} ms  over-50ms {sum(1 for v in s if v > 50)}")


conn, reply, total = [], [], []
slow = []
for i in range(N):
    t0 = time.perf_counter()
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(20)
    s.connect(SOCK)
    t1 = time.perf_counter()
    s.sendall((json.dumps({"id": "p", "method": METHOD, "params": PARAMS}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        c = s.recv(65536)
        if not c:
            break
        buf += c
    t2 = time.perf_counter()
    s.close()
    conn.append((t1 - t0) * 1000)
    reply.append((t2 - t1) * 1000)
    total.append((t2 - t0) * 1000)
    if (t2 - t0) * 1000 > 50:
        slow.append((i, (t1 - t0) * 1000, (t2 - t1) * 1000))
    time.sleep(GAP)

print(f"method {METHOD}  gap {GAP*1000:.0f} ms")
print(f"  connect      : {quantiles(conn)}")
print(f"  write->reply : {quantiles(reply)}")
print(f"  total        : {quantiles(total)}")
for i, c, r in slow[:20]:
    print(f"    slow round {i:4d}: connect {c:7.2f}  reply {r:7.2f}")
