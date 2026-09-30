#!/usr/bin/env python3
"""Whether Claude's wheel ramp carries across separate writes, how fast Claude repaints under a
continuous stream, and a finger through the old and new `PaneScroll` pumps.

`wheel-batch.py` (#567, #568) measured one write at a time. Its pump waits for the frame that
answered a write and then 45 ms more, so every write starts the ramp fresh — at most 12-15 writes
a second. This asks what happens when writes come closer than Claude's 40 ms reset: does the
per-report multiplier keep rising across them exactly as `Claude` below (the 2.1.285 window-mode
arithmetic, `Ndt` in the binary) says it should?

Throwaway named session, torn down at the end (#97). Usage: wheel-carry.py [LINES]
"""
import os, random, sys, threading, time, queue
from importlib import import_module

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
wb = import_module("wheel-batch")
composer, call, top, faithful, ramp = wb.composer, wb.call, wb.top, wb.faithful, wb.ramp
UP, DOWN = "\x1b[<64;40;20M", "\x1b[<65;40;20M"
STALLS = []


def send(pane, text):
    """`pane.send_text` the way the node makes it since #450: connect and send back to back on a
    non-blocking socket, so herdr's once-only look at a fresh connection (#445) sees it whole. The
    probe's plain `rpc` leaves a window there and stalls ~5 % of calls for 100 ms. This one still
    stalls more than the node's 0.25 % (#450) while `Frames` is busy parsing a repaint, because
    that thread can take the GIL between the connect and the send; every call over 50 ms is
    counted in STALLS so a row can be read with its stalls beside it."""
    import json, socket
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM | socket.SOCK_NONBLOCK)
    body = (json.dumps({"id": "kampr-probe", "method": "pane.send_text",
                        "params": {"pane_id": pane, "text": text}}) + "\n").encode()
    t = time.monotonic()
    s.connect(wb.SOCK)
    s.send(body)
    s.setblocking(True)
    s.settimeout(5)
    buf = b""
    while b"\n" not in buf:
        c = s.recv(65536)
        if not c:
            break
        buf += c
    s.close()
    took = (time.monotonic() - t) * 1000
    if took > 50:
        STALLS.append(took)


class Claude:
    """2.1.285's window (native) wheel arithmetic, fed report arrival times in ms."""

    def __init__(self):
        self.time, self.mult, self.dir, self.flip, self.wheel_mode, self.burst = -1e9, 1.0, 0, False, False, 0

    def report(self, t, d):
        if self.wheel_mode and t - self.time > 1500:
            self.wheel_mode, self.burst, self.mult = False, 0, 1.0
        if self.flip:
            self.flip = False
            if d != self.dir or t - self.time > 200:
                self.dir, self.time, self.mult = d, t, 1.0
                return 1
            self.wheel_mode = True
        gap = t - self.time
        if d != self.dir and self.dir != 0:
            self.flip, self.time = True, t
            return 0
        self.dir, self.time = d, t
        if self.wheel_mode:
            return None
        self.mult = 1.0 if gap > 40 else min(6.0, self.mult + 0.3)
        return int(self.mult)

    def write(self, t, d, k):
        return sum(self.report(t, d) for _ in range(k))


def carried(start_mult, k):
    """What k reports move when they continue a ramp at `start_mult` (None: fresh)."""
    rows, mult = 0, start_mult
    for _ in range(k):
        mult = 1.0 if mult is None else min(6.0, mult + 0.3)
        rows += int(mult)
    return rows, mult


def fit(rows, start_mult):
    k = 0
    while k < 40 and carried(start_mult, k + 1)[0] <= rows:
        k += 1
    return k


def home(pane):
    call("pane.send_text", {"pane_id": pane, "text": "\x1b[1;5F"})
    time.sleep(1.0)
    call("pane.send_text", {"pane_id": pane, "text": UP * 2})
    time.sleep(1.2)


def gaps_ms(stamps):
    return [(b - a) * 1000 for a, b in zip(stamps, stamps[1:])]


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else -1


def stream(pane, frames, k, gap, n):
    """n writes of k reports, one every `gap` ms on an absolute schedule."""
    home(pane)
    before = top(pane)
    oracle, sent, stalls = Claude(), [], len(STALLS)
    t0 = time.monotonic()
    for i in range(n):
        while time.monotonic() < t0 + i * gap / 1000:
            pass
        at = time.monotonic()
        send(pane, UP * k)
        sent.append(at)
    end = sent[-1]
    time.sleep(1.2)
    got = frames.since(t0)
    moved = before - top(pane)
    predicted = 0
    for s in sent:
        predicted += oracle.write((s - t0) * 1000, 1, k)
    fresh = n * ramp(k)
    during = [s for s in got if s <= end + 0.05]
    span = max(1e-3, (during[-1] - during[0])) if len(during) > 1 else 1e-3
    fg = gaps_ms(during)
    same, total = faithful(pane, frames)
    sg = gaps_ms(sent)
    print(f"  {n:>2} writes x{k} every {gap:>2}ms (sent p50 {pct(sg, .5):.0f} max {max(sg):.0f})  moved={moved:>3}"
          f"  carried-model={predicted:>3}  fresh-model={fresh:>3}  frames={len(got):>2}"
          f"  fps={(len(during) - 1) / span:.0f}  frame-gap p50={pct(fg, .5):.0f} max={max(fg or [0]):.0f}ms"
          f"  faithful={same}/{total}  stalls={len(STALLS) - stalls}")


class Link:
    """The phone's uplink and the node's serial write loop: writes delivered in order after a
    one-way latency of `base` + U(0, `jitter`) ms, and the `stall`th write held 100 ms the way
    herdr's socket holds one it looked at too early (#445) — everything behind it queues."""

    def __init__(self, pane, base=0.0, jitter=0.0, stall=None):
        self.pane, self.base, self.jitter, self.stall = pane, base, jitter, stall
        self.q, self.last, self.count = queue.Queue(), 0.0, 0
        self.arrivals = []
        threading.Thread(target=self.run, daemon=True).start()

    def send(self, text):
        at = max(self.last, time.monotonic() + (self.base + random.uniform(0, self.jitter)) / 1000)
        self.count += 1
        if self.count == self.stall:
            at += 0.100
        self.last = at
        self.q.put((at, text))

    def run(self):
        while True:
            at, text = self.q.get()
            while time.monotonic() < at:
                time.sleep(0.0005)
            self.arrivals.append(time.monotonic())
            send(self.pane, text)
            self.q.task_done()


QUIET, UNANSWERED = 0.045, 0.150
TICK, CARRY = 0.016, 0.024


def until(t):
    while time.monotonic() < t:
        time.sleep(0.0005)


def finger(pane, frames, rows, ms, pump, jitter=0.0, stall=None):
    """A finger travelling `rows` rows up over `ms`, through #568's pump ('fresh': every write
    starts the ramp, the next one 45 ms after the frame that answered it) or the new one
    ('carry': a write every TICK while the ramp is carried, then the same settle before a fresh
    one, counted from the tick that found nothing to carry). Mirrors `PaneScroll`'s pump."""
    home(pane)
    state = {"asked": 0, "done": None}

    def hand():
        start = time.monotonic()
        for i in range(1, rows + 1):
            until(start + i * ms / rows / 1000)
            state["asked"] = i
        state["done"] = time.monotonic()

    link = Link(pane, base=2.0, jitter=jitter, stall=stall)
    stalls = len(STALLS)
    before = top(pane)
    t0 = time.monotonic()
    threading.Thread(target=hand, daemon=True).start()
    box = {"sent": 0, "writes": 0, "carried": 0, "at": t0}

    def write(start):
        k = fit(state["asked"] - box["sent"], start)
        if k == 0:
            return None
        link.send(UP * k)
        got, mult = carried(start, k)
        box["sent"] += got
        box["writes"] += 1
        box["carried"] += start is not None
        box["at"] = time.monotonic()
        return mult

    def settle():
        at = box["at"]
        while time.monotonic() < at + UNANSWERED and not frames.since(at):
            time.sleep(0.001)
        answered = frames.since(at)
        if not answered:
            return until(at + UNANSWERED)
        until(max(answered[0], at + (TICK if pump == "carry" else 0)) + QUIET)

    while True:
        while state["asked"] - box["sent"] <= 0:
            if state["done"]:
                break
            time.sleep(0.001)
        if state["done"] and state["asked"] - box["sent"] <= 0:
            break
        mult = write(None)
        while pump == "carry" and mult is not None:
            until(box["at"] + TICK)
            if time.monotonic() - box["at"] > CARRY:
                break
            nxt = write(mult)
            if nxt is None:
                break
            mult = nxt
        settle()
    link.q.join()
    lifted = state["done"]
    time.sleep(1.2)
    moved = before - top(pane)
    during = [s for s in frames.since(t0) if s <= link.arrivals[-1] + 0.06]
    span = max(1e-3, during[-1] - during[0]) if len(during) > 1 else 1e-3
    fg = gaps_ms(during)
    tail = (link.arrivals[-1] - lifted) * 1000
    label = f"{pump}{' +stall' if stall else ''}"
    print(f"  {label:<12} jitter {jitter:>2.0f}ms  {rows:>3} rows in {ms:>4}ms: moved={moved:>3} writes={box['writes']:>2}"
          f" carried={box['carried']:>2} frames={len(during):>2} fps={(len(during) - 1) / span:>3.0f}"
          f" frame-gap p50={pct(fg, .5):.0f} max={max(fg or [0]):.0f}ms  last write {tail:.0f}ms after lift"
          f"  stalls={len(STALLS) - stalls}")
    return moved


def main():
    lines = int(sys.argv[1]) if len(sys.argv) > 1 else 500
    pane, frames, cwd = wb.transcript(lines)
    try:
        print("carry: n writes of k reports at a fixed gap (the ramp resets on a gap > 40 ms)")
        for k, gap, n in ((1, 10, 30), (1, 16, 30), (1, 20, 30), (1, 25, 30), (1, 30, 30), (1, 35, 30),
                          (1, 45, 20), (1, 60, 20), (2, 16, 20), (2, 20, 20), (3, 16, 12), (3, 20, 12),
                          (5, 20, 8)):
            stream(pane, frames, k, gap, n)
        print("a finger through each pump")
        for rows, ms in ((30, 250), (100, 250), (60, 1000), (150, 600), (20, 1000), (200, 1000)):
            for pump in ("fresh", "carry"):
                finger(pane, frames, rows, ms, pump)
                finger(pane, frames, rows, ms, pump)
        print("the same through a jittery link")
        for rows, ms in ((100, 250), (60, 1000), (150, 600)):
            for jitter in (10, 20, 30):
                finger(pane, frames, rows, ms, "carry", jitter)
        print("one write held 100 ms mid-drag (#445)")
        for rows, ms in ((100, 250), (150, 600)):
            for pump in ("fresh", "carry"):
                finger(pane, frames, rows, ms, pump, stall=5)
        frames.close()
        for _ in range(3):
            call("pane.send_text", {"pane_id": pane, "text": "\x03"})
            time.sleep(0.3)
    finally:
        composer.stop()
        import shutil
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
