#!/usr/bin/env python3
"""How far a Claude pane moves for k SGR wheel reports sent in ONE write, and how fast it answers.

A phone drag on a Claude pane is wheel reports (#387, #388). `PaneScroll` used to release them one
every 40 ms (#528), which caps the view at ~21 rows a second. This measures the alternative: a
whole batch in one `pane.send_text`. For each k it reads the top numbered line off herdr's
`pane.read visible` before and after, counts the `observe` frames the batch caused, times the
first and last of them, and compares the grid rebuilt from those frames against herdr's own
screen. Then two writes at a range of gaps (Claude's ramp resets after 40 ms), a finger driven
through both pumps — #528's and the batched, frame-gated one `PaneScroll` now runs — and the page
keys.

Throwaway named session, torn down at the end (#97). Usage: wheel-batch.py [ROUNDS]
"""
import base64, json, os, re, shutil, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from importlib import import_module
import vt

composer = import_module("composer-line")
NAME = f"kampr-probe-batch-{os.getpid()}"
HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]
call = composer.call
COLS, ROWS = composer.COLS, composer.ROWS
NUMBERED = re.compile(r"^\W*(\d+) - [a-z]")


class Frames:
    def __init__(self, pane, cols):
        self.screen = vt.Screen(cols, ROWS)
        self.lock = threading.Lock()
        self.stamps = []
        env = dict(os.environ, HERDR_SOCKET_PATH=SOCK)
        self.child = subprocess.Popen(
            ["herdr", "terminal", "session", "observe", pane, "--cols", str(cols), "--rows", str(ROWS)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, env=env)
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.child.stdout:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if rec.get("type") != "terminal.frame":
                continue
            data = base64.b64decode(rec["bytes"]).decode("utf-8", "replace")
            with self.lock:
                vt.feed(self.screen, data)
                self.stamps.append(time.monotonic())

    def since(self, t):
        with self.lock:
            return [s for s in self.stamps if s > t]

    def rows(self):
        with self.lock:
            return ["".join(r).rstrip() for r in self.screen.g]

    def close(self):
        self.child.kill()


def visible(pane):
    return call("pane.read", {"pane_id": pane, "source": "visible", "lines": 80, "format": "text"})["read"]["text"]


def top(pane):
    for line in visible(pane).splitlines():
        m = NUMBERED.match(line)
        if m:
            return int(m.group(1))
    return None


def faithful(pane, frames):
    ours = [r.rstrip() for r in frames.rows()]
    theirs = [r.rstrip() for r in visible(pane).splitlines()]
    theirs += [""] * (len(ours) - len(theirs))
    return sum(1 for a, b in zip(ours, theirs) if a == b), len(ours)


def trial(pane, frames, text, label):
    before = top(pane)
    t0 = time.monotonic()
    call("pane.send_text", {"pane_id": pane, "text": text})
    time.sleep(1.2)
    got = frames.since(t0)
    after = top(pane)
    same, total = faithful(pane, frames)
    first = f"{(got[0] - t0) * 1000:.0f}" if got else "-"
    last = f"{(got[-1] - t0) * 1000:.0f}" if got else "-"
    moved = (before - after) if before is not None and after is not None else None
    print(f"  {label:<12} top {before} -> {after}  moved={moved}  frames={len(got)}"
          f"  first={first}ms last={last}ms  faithful={same}/{total}")
    return moved, len(got)


def ramp(k):
    mult, rows = 1.0, 0
    for i in range(k):
        rows += int(mult)
        mult = min(6.0, mult + 0.3)
    return rows


def reports_for(rows, reversed_):
    count = 0
    while count < 40 and ramp(count + 1 - (1 if reversed_ else 0)) <= rows:
        count += 1
    return count


def finger(pane, frames, rows, ms, batched):
    """A finger travelling `rows` rows up over `ms`, through the new pump or #528's."""
    state = {"asked": 0, "done": False}

    def hand():
        for i in range(1, rows + 1):
            time.sleep(ms / rows / 1000)
            state["asked"] = i
        state["done"] = True

    before = top(pane)
    t0 = time.monotonic()
    threading.Thread(target=hand, daemon=True).start()
    sent_rows, writes, last = 0, 0, t0
    while not state["done"] or sent_rows < state["asked"]:
        pending = state["asked"] - sent_rows
        if pending <= 0:
            time.sleep(0.002)
            continue
        if batched:
            k = reports_for(pending, False)
            moved = ramp(k)
        else:
            k, moved = 1, 1
        at = time.monotonic()
        call("pane.send_text", {"pane_id": pane, "text": "\x1b[<64;40;20M" * k})
        sent_rows += moved
        writes += 1
        last = time.monotonic()
        if batched:
            deadline = at + 0.150
            while time.monotonic() < deadline and not frames.since(at):
                time.sleep(0.002)
            answered = frames.since(at)
            if answered:
                time.sleep(max(0, answered[0] + 0.045 - time.monotonic()))
        else:
            time.sleep(0.040)
    time.sleep(1.0)
    moved = before - top(pane)
    print(f"  {'batched' if batched else '1/40ms '} travel={rows} rows in {ms}ms: moved={moved} writes={writes}"
          f" last write {(last - t0) * 1000 - ms:.0f}ms after the finger stopped")


def paced(pane, frames, k, gap_ms, button, label):
    before = top(pane)
    t0 = time.monotonic()
    for _ in range(k):
        call("pane.send_text", {"pane_id": pane, "text": f"\x1b[<{button};40;20M"})
        time.sleep(gap_ms / 1000)
    time.sleep(1.0)
    got = frames.since(t0)
    after = top(pane)
    same, total = faithful(pane, frames)
    moved = (before - after) if before is not None and after is not None else None
    print(f"  {label:<12} top {before} -> {after}  moved={moved}  frames={len(got)}  faithful={same}/{total}")


def transcript(lines=300):
    """A real claude in the throwaway session with `lines` numbered lines on its transcript, and an
    observe stream on it. The caller owns `composer.stop()` and the returned directory."""
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-batch-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
    pane = ws["root_pane"]["pane_id"]
    time.sleep(1.0)
    call("pane.send_text", {"pane_id": pane, "text": "claude --model haiku\r"})
    for _ in range(60):
        time.sleep(0.5)
        text = visible(pane)
        if "trust" in text.lower():
            call("pane.send_text", {"pane_id": pane, "text": "\r"})
            time.sleep(4)
            break
        if "\n❯" in text:
            break
    call("pane.send_text", {"pane_id": pane, "text":
         f"Print the numbers 1 to {lines}, one per line, each followed by a dash and its English word. "
         "No tools, nothing else."})
    time.sleep(0.5)
    call("pane.send_text", {"pane_id": pane, "text": "\r"})
    for _ in range(lines):
        time.sleep(1)
        v = visible(pane)
        if f"{lines} - " in v and "esc to interrupt" not in v.lower():
            break
    time.sleep(2)
    cols = max(len(line) for line in visible(pane).splitlines())
    frames = Frames(pane, cols)
    time.sleep(1.5)
    print(f"claude {subprocess.run(['claude', '--version'], capture_output=True, text=True).stdout.strip()}"
          f"  observe {cols}x{ROWS}  top at rest {top(pane)}")
    return pane, frames, cwd


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 2
    pane, frames, cwd = transcript()
    try:
        up, down = "\x1b[<64;40;20M", "\x1b[<65;40;20M"
        home = lambda: (call("pane.send_text", {"pane_id": pane, "text": "\x1b[1;5F"}), time.sleep(1.0))
        for r in range(rounds):
            print(f"round {r + 1}: one write of k reports; ramp(k) is the predicted fresh-batch rows")
            for k in (1, 2, 3, 4, 5, 8, 10, 15, 20, 25, 30, 40):
                home()
                trial(pane, frames, up * 2, "prime up x2")
                trial(pane, frames, up * k, f"up x{k} ramp={ramp(k)}")
                trial(pane, frames, down * k, f"rev down x{k} ramp(k-1)={ramp(k - 1)}")
        print("two writes of 5 same direction, gap between them (fresh would be 12)")
        for gap in (5, 20, 35, 45, 60, 100):
            home()
            trial(pane, frames, up * 2, "prime up x2")
            before = top(pane)
            call("pane.send_text", {"pane_id": pane, "text": up * 5})
            time.sleep(gap / 1000)
            call("pane.send_text", {"pane_id": pane, "text": up * 5})
            time.sleep(1.2)
            print(f"  gap {gap}ms: moved {before - top(pane)}")
        print("a finger through each pump")
        for rows, ms in ((30, 250), (100, 250), (60, 1000), (150, 600)):
            for batched in (False, True):
                home()
                trial(pane, frames, up * 2, "prime up x2")
                finger(pane, frames, rows, ms, batched)
        print("one report per write, paced")
        for gap in (40, 16, 200):
            home()
            paced(pane, frames, 10, gap, 64, f"up 10@{gap}ms")
            paced(pane, frames, 10, gap, 65, f"down 10@{gap}ms")
        print("netting inside one write")
        home()
        trial(pane, frames, up * 20, "up x20")
        trial(pane, frames, up * 10 + down * 4, "up10 down4")
        print("keys")
        trial(pane, frames, "\x1b[5~", "PgUp")
        trial(pane, frames, "\x1b[5~", "PgUp")
        trial(pane, frames, "\x1b[6~", "PgDn")
        trial(pane, frames, "\x1b[1;5H", "ctrl+Home")
        trial(pane, frames, "\x1b[1;5F", "ctrl+End")
        frames.close()
        for _ in range(3):
            call("pane.send_text", {"pane_id": pane, "text": "\x03"})
            time.sleep(0.3)
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
