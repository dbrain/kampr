#!/usr/bin/env python3
"""The reply box and a real harness's box as one line, through a real node: the keys the client's
`ComposerMirror` would type for a run of edits go in as `input`, and each `convo.composer` that
comes back is compared with the box. Also times a key typed at the desk to the frame that carries
it (the desk poll), and a key typed through the node to the frame that carries it.

Stands up an isolated herdr and `kampr serve` with keystroke-latency/up.sh; tears both down.
Usage: round-trip.py [claude|codex|agy ...]
PIECE=400 cuts each line into writes of at most that many characters, as ComposerMirror does for a
node whose `keys` carry `paced: true`, and PASTE=<n> ends the run with a paste of about n
characters in four lines (#666).
"""
import json, os, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from rpc import rpc
from ws import WS
from common import CLEAN_ENV

ROOT = "/tmp/kmrt"
UP = os.path.join(os.path.dirname(HERE), "keystroke-latency")
BACK, LEFT, RIGHT, NEWLINE = "\x7f", "\x1b[D", "\x1b[C", "\n"
BOOT = {"claude": ("claude --model haiku", []), "codex": ("codex --no-daemon", ["\r"]), "agy": ("agy", ["\r"])}
MARK = {"claude": "❯", "codex": "›", "agy": ">"}


class Mirror:
    """ComposerMirror.typed, keystroke for keystroke."""

    def __init__(self):
        self.line, self.caret = "", 0

    def typed(self, before, after):
        shared = len(os.path.commonprefix([before, after]))
        a, b = before[shared:][::-1], after[shared:][::-1]
        tail = len(os.path.commonprefix([a, b]))
        removed = len(before) - shared - tail
        inserted = after[shared:len(after) - tail]
        out = []
        by = shared + removed - self.caret
        if by < 0:
            out.append(LEFT * -by)
        elif by > 0:
            out.append(RIGHT * by)
        if removed:
            out.append(BACK * removed)
        cut = int(os.environ.get("PIECE", "0")) or len(after) + 1
        for i, piece in enumerate(inserted.split("\n")):
            if i:
                out.append(NEWLINE)
            out += [piece[at:at + cut] for at in range(0, len(piece), cut)]
        self.line, self.caret = after, shared + len(inserted)
        return out


def seen(text, box):
    """The box read back whole, or, scrolled past its top, its last words (#666)."""
    words = lambda t: " ".join(t.split())
    return text == box or (os.environ.get("PASTE") and len(text) < len(box) and words(text)
                           and words(box).endswith(words(text)))


class Feed:
    def __init__(self, ws):
        self.ws, self.log, self.lock = ws, [], threading.Lock()
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        while True:
            try:
                _op, payload = self.ws.frame()
            except Exception:
                return
            msg = json.loads(payload)
            if msg.get("t") == "convo.composer":
                with self.lock:
                    self.log.append((time.perf_counter(), msg))

    def wait(self, t0, want, seconds=4.0):
        end = time.perf_counter() + seconds
        while time.perf_counter() < end:
            with self.lock:
                for at, m in self.log:
                    if at >= t0 and seen(m.get("text") or "", want) and "keys" in m:
                        return at, m
            time.sleep(0.003)
        with self.lock:
            return None, (self.log[-1][1] if self.log else None)


def run(harness, cfg, call):
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-rt-XXXXXX"], capture_output=True, text=True).stdout.strip()
    ws_ = call("workspace.create", {"label": harness, "cwd": cwd, "focus": False})
    local = ws_["root_pane"]["pane_id"]
    send = lambda text: call("pane.send_text", {"pane_id": local, "text": text})
    screen = lambda: call("pane.read", {"pane_id": local, "source": "visible", "format": "text",
                                        "strip_ansi": True})["read"]["text"]
    cmd, settle = BOOT[harness]
    time.sleep(1)
    send(cmd + "\r")
    for _ in range(80):
        time.sleep(0.5)
        s = screen()
        if harness == "claude" and "trust this folder" in s:
            send("\x1b[B"); time.sleep(0.4); send("\r"); time.sleep(3); continue
        if any(l.startswith(MARK[harness]) for l in s.splitlines()[-12:]):
            break
    for key in settle:
        time.sleep(6); send(key)
    time.sleep(8)

    ws = WS("127.0.0.1", int(cfg["PORT"]), protocol=f"kampr.token.{cfg['TOKEN']}")
    pane = None
    while pane is None:
        _op, payload = ws.frame()
        msg = json.loads(payload)
        if msg.get("t") == "herd":
            pane = next((p["id"] for p in msg["panes"] if p["id"].endswith("/" + local)), None)
    if os.environ.get("DEBUG"):
        print(screen()[-1500:])
        print([ (p["id"], p.get("agent"), p.get("agent_status")) for p in msg["panes"]])
    feed = Feed(ws)
    ws.send(json.dumps({"t": "watch", "pane": pane, "conversation": True}))
    t0 = time.perf_counter()
    at, first = feed.wait(t0, "", 10)
    print(f"\n== {harness}: first frame {first}")

    desk = []
    typed = ""
    for ch in "fix the build":
        typed += ch
        t0 = time.perf_counter()
        send(ch)
        at, _ = feed.wait(t0, typed, 3)
        desk.append(None if at is None else (at - t0) * 1000)
        time.sleep(0.15)
    got = sorted(round(x) for x in desk if x is not None)
    print(f"  desk key -> convo.composer: {len(got)}/{len(desk)}  ms {got}")
    send(BACK * 40); time.sleep(1.5)

    mirror = Mirror()
    box = ""
    steps = [("type", c) for c in "push the branch"] + ([("type", c) for c in "\n\nthen tag it"] if os.environ.get("PARAGRAPHS") else []) + [
        ("edit", "push now the branch"), ("edit", "push the branch"), ("edit", "push the main branch"),
        ("edit", "push the main branch\nand tag it"), ("edit", "push the main branch\nand tag"),
        ("edit", "git push the main branch\nand tag"),
    ]
    if os.environ.get("PASTE"):
        per = int(os.environ["PASTE"]) // 4
        words = "the quick brown fox jumps over the lazy dog while the kettle boils "
        lines = [(f"{i}:" + words * (per // len(words) + 1))[:per].rstrip() for i in range(4)]
        steps.append(("edit", "git push the main branch\nand tag\n" + "\n".join(lines)))
    through = []
    ok = 0
    for kind, what in steps:
        after = box + what if kind == "type" else what
        keys = mirror.typed(box, after)
        t0 = time.perf_counter()
        for k in keys:
            ws.send(json.dumps({"t": "input", "pane": pane, "text": k}))
        box = after
        at, last = feed.wait(t0, box, 4 + len(box) / 1000)
        if at is None:
            print(f"  MISMATCH after {kind} {what!r}: box {box!r} pane {last and last.get('text')!r} caret {last and last.get('caret')}")
            break
        ok += 1
        through.append((at - t0) * 1000)
        if last.get("caret") != mirror.caret:
            print(f"  caret: pane {last.get('caret')} mirror {mirror.caret} after {what!r}")
    print(f"  box -> pane -> convo.composer: {ok}/{len(steps)} in step; ms p50 "
          f"{sorted(through)[len(through)//2]:.0f} max {max(through):.0f}" if through else "  nothing came back")
    if os.environ.get("PASTE") and ok == len(steps) and harness == "claude":
        time.sleep(0.5)
        ws.send(json.dumps({"t": "input", "pane": pane, "text": "\r"}))
        record = None
        for _ in range(40):
            time.sleep(0.5)
            for f in __import__("glob").glob(os.path.expanduser("~/.claude/projects/" + cwd.replace("/", "-") + "/*.jsonl")):
                for line in open(f):
                    r = json.loads(line)
                    if r.get("type") == "user" and not r.get("isMeta"):
                        c = r["message"]["content"]
                        record = c if isinstance(c, str) else "".join(b.get("text", "") for b in c if isinstance(b, dict))
            if record:
                break
        print(f"  submitted: {'verbatim' if record == box else 'pasted' if record and '<pasted_content' in record else repr(record)[:200]}")
    send(BACK * 80); time.sleep(1)
    ws.close()
    subprocess.run(["rm", "-rf", cwd])


def main():
    env = dict(CLEAN_ENV, NAME="kamprrt")
    subprocess.run(["bash", os.path.join(UP, "up.sh"), ROOT], env=env, check=True)
    cfg = {}
    for line in open(os.path.join(ROOT, "env")):
        k, v = line.replace("export ", "").strip().split("=", 1)
        cfg[k] = v.strip('"')
    call = lambda m, p=None: rpc(m, p or {}, sock_path=cfg["HERDR_SOCKET_PATH"])["result"]
    try:
        for harness in sys.argv[1:] or ["claude", "codex", "agy"]:
            try:
                run(harness, cfg, call)
            except Exception:
                import traceback; traceback.print_exc()
    finally:
        subprocess.run(["bash", os.path.join(UP, "down.sh"), ROOT], env=env)


if __name__ == "__main__":
    main()
