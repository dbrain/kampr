"""The same round trip with no Kampr on the path at all: `pane.send_text` over herdr's own
socket, and `herdr terminal session observe` on the other side. #273's experiment, re-run."""
import base64, json, os, select, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from rpc import rpc


def quantiles(s):
    s = sorted(s)
    def q(p):
        return s[round((len(s) - 1) * p)]
    return (f"n={len(s)} min {s[0]:.1f}  p50 {q(.5):.1f}  p75 {q(.75):.1f}  p90 {q(.9):.1f}  "
            f"max {s[-1]:.1f} ms")


def histogram(values, width=10.0, top=200.0):
    b = {}
    for v in values:
        k = min(int(v // width) * width, top)
        b[k] = b.get(k, 0) + 1
    return "  ".join(f"[{int(k)}-{int(k+width)}){b[k]}" for k in sorted(b))


def main():
    rounds = int(os.environ.get("ROUNDS", "120"))
    pane = rpc("pane.list", {})["result"]["panes"][0]
    local = pane["pane_id"]
    rows = pane["scroll"]["viewport_rows"]
    obs = subprocess.Popen(
        ["herdr", "terminal", "session", "observe", local, "--cols", "120", "--rows", str(rows)],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    os.set_blocking(obs.stdout.fileno(), False)
    buf = b""

    def drain(seconds):
        end = time.perf_counter() + seconds
        while True:
            left = end - time.perf_counter()
            if left <= 0:
                return
            r, _, _ = select.select([obs.stdout], [], [], left)
            if not r:
                return
            obs.stdout.read()
            end = time.perf_counter() + seconds

    drain(0.8)
    readings, trace = [], []
    for i in range(rounds):
        ch = chr(ord("a") + i % 26)
        want = ch.encode()
        t0 = time.perf_counter()
        rpc("pane.send_text", {"pane_id": local, "text": ch})
        sent = time.perf_counter()
        seen = None
        deadline = t0 + 10
        while seen is None and time.perf_counter() < deadline:
            r, _, _ = select.select([obs.stdout], [], [], deadline - time.perf_counter())
            now = time.perf_counter()
            if not r:
                break
            chunk = obs.stdout.read()
            if chunk:
                buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue
                if rec.get("type") == "terminal.frame" and want in base64.b64decode(rec["bytes"]):
                    seen = now
                    break
        if seen is None:
            raise SystemExit(f"round {i}: {ch} never came back")
        readings.append((seen - t0) * 1000)
        trace.append((i, (sent - t0) * 1000, (seen - t0) * 1000))
        rpc("pane.send_keys", {"pane_id": local, "keys": ["Backspace"]})
        drain((50 + (i * 37) % 190) / 1000.0)
    print(f"herdr alone, send_text -> observe frame: {quantiles(readings)}")
    print(f"  {histogram(readings)}")
    if os.environ.get("TRACE"):
        for i, rpc_ms, ms in trace:
            print(f"    round {i:3d}  rpc {rpc_ms:6.1f}  total {ms:7.1f}")
    obs.kill()


main()
