"""Probe #442 follow-up: where the ~110 ms in a keystroke round trip actually is.

One keystroke, three clocks, all in one single-threaded process:

  t0   the client's `input` leaves userspace
  tk   the kernel timestamps the TCP segment carrying the answering `grid.patch` (SO_TIMESTAMPNS)
  tu   this process wakes up and reads it
  to   a `herdr terminal session observe` on the same pane, read in the same select loop,
       first shows the character

`tk - t0` is everything outside this client. `tu - tk` is this client's own wakeup — the quantity
#442 guessed at. `to - t0` is herdr's own share, measured on the same keystroke rather than in a
separate run.
"""
import json, os, select, socket, struct, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from rpc import rpc
from ws import WS

SO_TIMESTAMPNS = 35
SCM_TIMESTAMPNS = 35


class Timed(WS):
    """A WS whose reads carry the kernel's arrival timestamp for the segment they came from."""

    def enable_timestamps(self):
        self.sock.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS, 1)
        self.kernel_at = None
        self.boot_offset = time.clock_gettime(time.CLOCK_REALTIME) - time.perf_counter()

    def _fill(self, n):
        while len(self.buf) < n:
            data, anc, _flags, _addr = self.sock.recvmsg(65536, socket.CMSG_SPACE(32))
            if not data:
                raise ConnectionError("closed")
            for level, ctype, payload in anc:
                if level == socket.SOL_SOCKET and ctype == SCM_TIMESTAMPNS:
                    sec, nsec = struct.unpack("qq", payload[:16])
                    self.kernel_at = (sec + nsec / 1e9) - self.boot_offset
            self.buf += data


def quantiles(values):
    s = sorted(values)
    if not s:
        return "no readings"
    def q(p):
        return s[round((len(s) - 1) * p)]
    return (
        f"n={len(s)} min {s[0]:.1f}  p50 {q(.5):.1f}  p75 {q(.75):.1f}  p90 {q(.9):.1f}  "
        f"p95 {q(.95):.1f}  max {s[-1]:.1f} ms"
    )


def histogram(values, width=20.0, top=260.0):
    buckets = {}
    for v in values:
        b = min(int(v // width) * width, top)
        buckets[b] = buckets.get(b, 0) + 1
    return "  ".join(f"[{int(b)}-{int(b + width)}){buckets[b]}" for b in sorted(buckets))


def main():
    port = int(os.environ["PORT"])
    token = os.environ["TOKEN"]
    rounds = int(os.environ.get("ROUNDS", "200"))
    bare = os.environ.get("BARE", "1") == "1"
    with_observe = os.environ.get("OBSERVE", "1") == "1"

    ws = Timed("127.0.0.1", port, protocol=f"kampr.token.{token}")
    ws.enable_timestamps()

    pane = None
    while pane is None:
        _op, payload = ws.frame()
        msg = json.loads(payload)
        if msg.get("t") == "herd":
            pane = msg["panes"][0]["id"]
    local = pane.rsplit("/", 1)[-1]
    info = rpc("pane.list", {})["result"]["panes"][0]
    cols, rows = 120, info["scroll"]["viewport_rows"]
    geom = rpc("pane.get", {"pane_id": local})
    if geom and "result" in geom:
        pass

    ws.send(json.dumps({"t": "watch", "pane": pane}))

    if bare:
        ws.send(json.dumps({"t": "input", "pane": pane, "text": "stty sane; exec cat\n"}))
        for _ in range(600):
            info = rpc("pane.process_info", {"pane_id": local})
            procs = info["result"]["process_info"]["foreground_processes"]
            if any(p["name"] == "cat" for p in procs):
                break
            time.sleep(0.1)
        else:
            raise SystemExit("the pane never got a shell-free process on it")

    observer = None
    if with_observe:
        observer = subprocess.Popen(
            ["herdr", "terminal", "session", "observe", local, "--cols", str(cols), "--rows", str(rows)],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env={**os.environ},
        )
        os.set_blocking(observer.stdout.fileno(), False)

    def drain(seconds):
        # Bounded: a node with anything else to say — a herd patch, a keepalive — would otherwise
        # keep resetting the quiet window and the round would never end.
        hard = time.perf_counter() + 3 * seconds
        end = time.perf_counter() + seconds
        while True:
            left = min(end, hard) - time.perf_counter()
            if left <= 0:
                return
            fds = [ws.sock]
            if observer:
                fds.append(observer.stdout)
            r, _, _ = select.select(fds, [], [], left)
            if not r:
                return
            for f in r:
                if f is ws.sock:
                    ws.frame()
                else:
                    observer.stdout.read()
            end = time.perf_counter() + seconds

    drain(0.6)

    started = time.perf_counter()
    outside, wakeup, total, herdr_share, trace = [], [], [], [], []
    obuf = b""
    for round_no in range(rounds):
        ch = chr(ord("a") + round_no % 26)
        want = ch.encode()
        ws.kernel_at = None
        t0 = time.perf_counter()
        ws.send(json.dumps({"t": "input", "pane": pane, "text": ch}))
        seen_ws = seen_obs = None
        deadline = t0 + 10.0
        while (seen_ws is None or (observer and seen_obs is None)) and time.perf_counter() < deadline:
            fds = []
            if seen_ws is None:
                fds.append(ws.sock)
            if observer and seen_obs is None:
                fds.append(observer.stdout)
            r, _, _ = select.select(fds, [], [], max(0.0, deadline - time.perf_counter()))
            now = time.perf_counter()
            for f in r:
                if f is ws.sock:
                    while time.perf_counter() < deadline:
                        _op, payload = ws.frame()
                        arrived = ws.kernel_at
                        msg = json.loads(payload)
                        if msg.get("t") in ("grid.patch", "grid.reset") and msg.get("pane") == pane:
                            if ch in payload.decode(errors="replace"):
                                seen_ws = (arrived, now)
                                break
                        if not ws.pending():
                            break
                else:
                    chunk = observer.stdout.read()
                    if chunk:
                        obuf += chunk
                    while b"\n" in obuf:
                        line, obuf = obuf.split(b"\n", 1)
                        try:
                            rec = json.loads(line)
                        except ValueError:
                            continue
                        if rec.get("type") != "terminal.frame":
                            continue
                        import base64
                        if want in base64.b64decode(rec["bytes"]):
                            seen_obs = now
        if seen_ws is None:
            print(f"  !! round {round_no}: {ch!r} never came back inside 10 s; stopping at "
                  f"{len(total)} readings", flush=True)
            break
        arrived, noticed = seen_ws
        total.append((noticed - t0) * 1000)
        if arrived is not None:
            outside.append((arrived - t0) * 1000)
            wakeup.append((noticed - arrived) * 1000)
        if seen_obs is not None:
            herdr_share.append((seen_obs - t0) * 1000)
        trace.append((round_no, (noticed - t0) * 1000,
                      None if seen_obs is None else (seen_obs - t0) * 1000))

        ws.send(json.dumps({"t": "input", "pane": pane, "keys": ["Backspace"]}))
        drain((50 + (round_no * 37) % 190) / 1000.0)
        if os.environ.get("HEARTBEAT") and round_no % 10 == 9:
            print(f"  .. {round_no + 1} rounds, {time.perf_counter() - started:.1f}s", flush=True)

    print(f"total   client send -> client notices : {quantiles(total)}")
    print(f"  {histogram(total)}")
    print(f"outside client send -> kernel has it  : {quantiles(outside)}")
    print(f"  {histogram(outside)}")
    print(f"wakeup  kernel has it -> client reads : {quantiles(wakeup)}")
    print(f"  {histogram(wakeup)}")
    if herdr_share:
        print(f"herdr   client send -> observe frame  : {quantiles(herdr_share)}")
        print(f"  {histogram(herdr_share)}")
    if os.environ.get("TRACE"):
        for i, tot, obs in trace:
            print(f"    round {i:3d}  total {tot:7.1f}  observe {'-' if obs is None else f'{obs:7.1f}'}")
    if observer:
        observer.kill()
    ws.close()


if __name__ == "__main__":
    main()
