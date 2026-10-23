#!/usr/bin/env python3
"""What answering the kitty keyboard query and the colour queries changes about Claude Code, played
from a pty this script owns and answers as kampr-native would.

    VENV/bin/python research/probe/claude-kitty.py claude [fullscreen|default ...]
    VENV/bin/python research/probe/claude-kitty.py herdr

`claude` runs three variants against the mock Messages endpoint of claude-render.py:
  A  the kitty query and OSC 10/11/12 unanswered (kampr-native before alacritty)
  B  `CSI ? u` answered with the flags pushed so far, push/pop/set tracked
  C  B plus OSC 10/11/12 and OSC 4 answered from the Soft palette
and logs every query, the flags pushed, screens at three checkpoints, and what each legacy and
kitty key does to the composer. `herdr` asks herdr 0.9.5 the same kitty questions from a pane
under an empty HOME, and logs what the pane receives for API keys and for a client's keys.
Needs pyte (claude-replay.py says how).
"""

import importlib.util
import http.server
import json
import os
import pty
import fcntl
import re
import select
import shutil
import signal
import struct
import sys
import tempfile
import termios
import threading
import time

import pyte

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
spec = importlib.util.spec_from_file_location("claude_render", os.path.join(HERE, "claude-render.py"))
cr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cr)

COLS, ROWS = 100, 30
FG, BG = (0xDD, 0xE3, 0xEE), (0x0B, 0x0D, 0x12)
SOFT = [0x1B1F27, 0xF2707A, 0x5FD68F, 0xE9C46A, 0x7FB0FF, 0xC79BE0, 0x5FD2D2, 0xC6CCD9,
        0x7C8496, 0xFF8E96, 0x86E9AB, 0xF7D98C, 0xA6C8FF, 0xDDB8F0, 0x8CE7E7, 0xF2F5FA]
STEPS = [0, 95, 135, 175, 215, 255]


def slot(n):
    if n < 16:
        v = SOFT[n]
        return (v >> 16, (v >> 8) & 255, v & 255)
    if n < 232:
        i = n - 16
        return (STEPS[i // 36], STEPS[(i // 6) % 6], STEPS[i % 6])
    v = 8 + (n - 232) * 10
    return (v, v, v)


def rgb(c):
    return "rgb:" + "/".join(f"{x:02x}{x:02x}" for x in c)


QUERY = re.compile(rb"\x1b\[([?>=<]?)([0-9;:]*)([ -/]*)([@-~])|\x1b\]([^\x07\x1b]*)(\x07|\x1b\\)|\x1b[P_]([^\x1b]*)\x1b\\")


class Term:
    def __init__(self, variant, tui, port):
        self.variant = variant
        self.work = tempfile.mkdtemp(prefix="kck-", dir=os.environ.get("XDG_RUNTIME_DIR"))
        cfg, cwd = os.path.join(self.work, "cfg"), os.path.join(self.work, "proj")
        os.makedirs(cfg)
        os.makedirs(cwd)
        with open(os.path.join(cfg, ".claude.json"), "w") as f:
            json.dump({"hasCompletedOnboarding": True, "numStartups": 5,
                       "customApiKeyResponses": {"approved": [cr.KEY[-20:]], "rejected": []},
                       "projects": {cwd: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}}, f)
        with open(os.path.join(cfg, "settings.json"), "w") as f:
            json.dump({"tui": tui}, f)
        env = {k: v for k, v in os.environ.items() if not k.startswith(("CLAUDE", "HERDR_", "ANTHROPIC", "KITTY", "TERM_PROGRAM", "GHOSTTY", "WEZTERM", "COLORFGBG"))}
        env |= {"CLAUDE_CONFIG_DIR": cfg, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{port}", "ANTHROPIC_API_KEY": cr.KEY,
                "DISABLE_TELEMETRY": "1", "DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                "TERM": "xterm-256color", "COLORTERM": "truecolor"}
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execvpe("claude", ["claude"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.raw = b""
        self.pending = b""
        self.queries = []
        self.flags = [0]
        self.kitty = []
        self.modes = {}
        self.phase = "boot"

    def answer(self, m):
        q = m.group(0)
        if m.group(4) is not None:
            pre, params, inter, fin = m.group(1), m.group(2), m.group(3), m.group(4)
            p = [int(x) if x.isdigit() else 0 for x in params.decode().split(";")] if params else []
            p0 = p[0] if p else 0
            if fin == b"h" and pre == b"?" or fin == b"l" and pre == b"?":
                for x in p:
                    self.modes[x] = fin == b"h"
                return None
            if fin == b"u" and pre in (b">", b"<", b"="):
                self.kitty.append((round(time.time() - self.t0, 2), self.phase, q))
                if self.variant == "A":
                    return None
                if pre == b">":
                    self.flags.append(p0)
                elif pre == b"<":
                    for _ in range(max(p0, 1)):
                        if len(self.flags) > 1:
                            self.flags.pop()
                else:
                    how = p[1] if len(p) > 1 else 1
                    cur = self.flags[-1]
                    self.flags[-1] = p0 if how == 1 else cur | p0 if how == 2 else cur & ~p0
                return None
            if fin == b"c" and pre == b"" and p0 == 0:
                return self.log(q, b"\x1b[?62;22c")
            if fin == b"c" and pre == b">" and p0 == 0:
                return self.log(q, b"\x1b[>0;0;0c")
            if fin == b"q" and pre == b">" and inter == b"":
                return self.log(q, b"\x1bP>|kampr 0.1.118\x1b\\")
            if fin == b"n" and p0 == 5 and pre == b"":
                return self.log(q, b"\x1b[0n")
            if fin == b"n" and p0 == 6:
                y, x = self.screen.cursor.y + 1, self.screen.cursor.x + 1
                return self.log(q, f"\x1b[?{y};{x};1R".encode() if pre == b"?" else f"\x1b[{y};{x}R".encode())
            if fin == b"u" and pre == b"?":
                return self.log(q, None if self.variant == "A" else f"\x1b[?{self.flags[-1]}u".encode())
            if fin == b"p" and inter == b"$":
                if pre == b"?":
                    known = {1, 25, 1049, 1000, 1002, 1003, 1006, 2004, 1004}
                    st = (1 if self.modes.get(p0, p0 == 25) else 2) if p0 in known else 0
                    return self.log(q, f"\x1b[?{p0};{st}$y".encode())
                return self.log(q, f"\x1b[{p0};0$y".encode())
            if fin == b"t" and p0 in (14, 16, 18):
                return self.log(q, f"\x1b[8;{ROWS};{COLS}t".encode() if p0 == 18 else None)
            if fin == b"m" and pre in (b"?",):
                return self.log(q, None)
            return None
        if m.group(5) is not None:
            body, term = m.group(5), m.group(6)
            if b"?" not in body:
                return None
            parts = body.split(b";")
            reply = None
            if self.variant == "C":
                if parts[0] in (b"10", b"11", b"12") and parts[1:] == [b"?"]:
                    c = FG if parts[0] != b"11" else BG
                    reply = b"\x1b]" + parts[0] + b";" + rgb(c).encode() + term
                elif parts[0] == b"4" and len(parts) >= 3:
                    out = b""
                    for i in range(1, len(parts) - 1, 2):
                        if parts[i + 1] == b"?":
                            n = int(parts[i])
                            out += b"\x1b]4;" + parts[i] + b";" + rgb(slot(n)).encode() + term
                    reply = out or None
            return self.log(q, reply)
        return self.log(q, None)

    def log(self, q, reply):
        self.queries.append((round(time.time() - self.t0, 2), q, reply))
        return reply

    def pump(self, secs, until=None):
        end = time.time() + secs
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if not r:
                continue
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return False
            self.raw += chunk
            data = self.pending + chunk
            at = 0
            for m in QUERY.finditer(data):
                self.stream.feed(PYTE_UNSAFE.sub(b"", data[at:m.end()]))
                at = m.end()
                reply = self.answer(m)
                if reply:
                    os.write(self.fd, reply)
            tail = data[at:]
            cut = tail.rfind(b"\x1b")
            if cut >= 0 and len(tail) - cut < 64:
                self.stream.feed(PYTE_UNSAFE.sub(b"", tail[:cut]))
                self.pending = tail[cut:]
            else:
                self.stream.feed(PYTE_UNSAFE.sub(b"", tail))
                self.pending = b""
            if until and until in self.raw[-6000:]:
                return True
        return False

    def type(self, text, gap=0.01):
        for ch in text:
            os.write(self.fd, ch.encode())
            time.sleep(gap)

    def key(self, b):
        os.write(self.fd, b)

    def lines(self):
        return [l.rstrip() for l in self.screen.display]

    def composer(self):
        ls = self.lines()
        idx = [i for i, l in enumerate(ls) if "❯" in l or l.lstrip().startswith(">")]
        if not idx:
            return ["<no prompt glyph>"] + [l for l in ls if l.strip()][-4:]
        i = idx[-1]
        out = []
        for l in ls[i:i + 6]:
            if l.strip().startswith(("╰", "─")) and out:
                break
            out.append(l)
        return out

    def styles(self):
        seen = set()
        for row in self.screen.buffer.values():
            for ch in row.values():
                seen.add((ch.fg, ch.bg, ch.bold, ch.italics, ch.underscore, ch.reverse, ch.strikethrough))
        return seen

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass
        shutil.rmtree(self.work, ignore_errors=True)


VOLATILE = [(re.compile(r"\d+(\.\d+)?\s*(s|ms|m)\b"), "<t>"), (re.compile(r"\d[\d,.]*k?\s*tokens"), "<n> tokens"),
            (re.compile(r"[✻✶✳✢·✽*⏺◐◓◑◒⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]"), "<s>"), (re.compile(r"\b[A-Z][a-z]+ing…"), "<verb>…"),
            (re.compile(r"\b[A-Z][a-zé]+(ed|ing) for\b"), "<verb> for"),
            (re.compile(r"\d+:\d\d [ap]m"), "<clock>"), (re.compile(r"kck-[a-z0-9_]+"), "kck-<tmp>")]


def norm(lines):
    out = []
    for l in lines:
        for rx, sub in VOLATILE:
            l = rx.sub(sub, l)
        out.append(l)
    return out


PYTE_UNSAFE = re.compile(rb"\x1b\[\?[0-9;]*n|\x1b\[[<>=][0-9;]*u|\x1b_[^\x1b]*\x1b\\")
ANY_CSI = re.compile(rb"\x1b\[[?>=<]?[0-9;:]*[ -/]*[@-~]")
SGR = re.compile(rb"\x1b\[[0-9;:]*m")


def session(variant, tui, port):
    t = Term(variant, tui, port)
    t.t0 = time.time()
    shots = {}
    keys = []
    try:
        t.pump(15, until=b"\xe2\x9d\xaf")
        t.pump(2.5)
        shots["boot"] = norm(t.lines())
        sty = {"boot": t.styles()}
        boot_queries = list(t.queries)
        boot_kitty = list(t.kitty)
        t.phase = "turn1"
        t.type("LONGANSWER please")
        t.pump(0.8)
        t.key(b"\r")
        t.pump(40, until=b"REPLYEND")
        t.pump(4)
        shots["reply"] = norm(t.lines())
        sty["reply"] = t.styles()
        t.phase = "turn2"
        t.type("SHORT one")
        t.pump(0.8)
        t.key(b"\r")
        t.pump(15, until=b"Short answer")
        t.pump(3)
        shots["second"] = norm(t.lines())
        sty["second"] = t.styles()
        t.phase = "keys"
        sgr = set(SGR.findall(t.raw))
        csi = set(ANY_CSI.findall(t.raw))
        if os.environ.get("KCK_SAVE"):
            with open(os.path.join(os.environ["KCK_SAVE"], f"kck-{tui}-{variant}.bin"), "wb") as f:
                f.write(t.raw)

        def probe(name, setup, key, wait=0.8):
            t.key(b"\x03")
            t.pump(0.5)
            if setup:
                t.type(setup)
                t.pump(0.6)
            before = t.composer()
            for k in key if isinstance(key, list) else [key]:
                t.key(k)
                t.pump(0.35)
            t.pump(wait)
            keys.append((name, before, t.composer()))

        probe("backspace \\x7f", "hello", b"\x7f")
        probe("ctrl-c \\x03 on a typed line", "typed line", b"\x03")
        probe("kitty ctrl-c CSI 99;5u", "typed line", b"\x1b[99;5u")
        probe("Esc Esc legacy", "abc text", [b"\x1b", b"\x1b"])
        probe("Esc Esc kitty CSI 27u", "abc text", [b"\x1b[27u", b"\x1b[27u"])
        probe("alt-enter \\x1b\\r", "line1", b"\x1b\r")
        probe("shift-enter kitty CSI 13;2u", "line1", b"\x1b[13;2u")
        probe("ctrl-j \\n", "line1", b"\n")
        probe("arrow up \\x1b[A", "", b"\x1b[A")
        probe("arrow up then down", "", [b"\x1b[A", b"\x1b[B"])
        t.key(b"\x03")
        t.pump(0.5)
        t.type("LONGANSWER again")
        t.pump(0.6)
        t.key(b"\r")
        t.pump(1.5)
        t.key(b"\x1b")
        t.pump(3)
        interrupted = "Interrupted" in "\n".join(t.lines()) or b"Interrupted" in t.raw[-20000:]
        keys.append(("Esc \\x1b while streaming", ["<streaming>"], ["Interrupted shown" if interrupted else "no 'Interrupted'"]))
        t.pump(2)
        mark = len(t.raw)
        t.phase = "exit"
        t.key(b"\x03")
        t.pump(0.5)
        t.key(b"\x03")
        t.pump(3)
        tail = t.raw[mark:]
        exit_kitty = re.findall(rb"\x1b\[[<>=][0-9;]*u", tail)
        alive = os.waitpid(t.pid, os.WNOHANG) == (0, 0)
        return dict(shots=shots, boot_queries=boot_queries, boot_kitty=boot_kitty, all_queries=t.queries,
                    kitty=t.kitty, keys=keys, sgr=sgr, csi=csi, styles=sty, exit_kitty=exit_kitty, alive_after_exit=alive,
                    flags=t.flags)
    finally:
        t.close()


def show(r):
    def q(x):
        return x.decode("latin1").encode("unicode_escape").decode() if x is not None else None
    print("  boot queries:")
    for at, qq, rep in r["boot_queries"]:
        print(f"    +{at}s {q(qq)} -> {q(rep)}")
    later = r["all_queries"][len(r["boot_queries"]):]
    print(f"  later queries: {[(q(a), q(b)) for _, a, b in later]}")
    print(f"  kitty push/pop/set written: {[(a, ph, q(x)) for a, ph, x in r['kitty']]}; final stack {r['flags']}")
    print(f"  on exit (ctrl-c twice): kitty seqs {[q(x) for x in r['exit_kitty']]}, still alive {r['alive_after_exit']}")
    for name, before, after in r["keys"]:
        print(f"  key {name}:\n      before {before}\n      after  {after}")


class Quiet(http.server.ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass


def claude(tuis):
    server = Quiet(("127.0.0.1", 0), cr.Mock)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    for tui in tuis or ["fullscreen", "default"]:
        res = {}
        for v in "ABC":
            print(f"=== tui={tui} variant {v}")
            res[v] = session(v, tui, port)
            show(res[v])
        for a, b in (("A", "B"), ("B", "C"), ("A", "C")):
            for shot in ("boot", "reply", "second"):
                x, y = res[a]["shots"][shot], res[b]["shots"][shot]
                diff = [(i, l, m) for i, (l, m) in enumerate(zip(x, y)) if l != m]
                print(f"  {a} vs {b} @{shot}: {len(diff)} differing rows")
                for i, l, m in diff[:8]:
                    print(f"     row {i}\n       {a}: {l!r}\n       {b}: {m!r}")
            only_a = res[a]["sgr"] - res[b]["sgr"]
            only_b = res[b]["sgr"] - res[a]["sgr"]
            print(f"  {a} vs {b} SGR sequences only in {a}: {sorted(only_a)[:12]}; only in {b}: {sorted(only_b)[:12]}")
            norm_csi = lambda xs: {re.sub(rb"\d+", b"n", x) for x in xs}
            print(f"  {a} vs {b} CSI shapes only in {a}: {sorted(norm_csi(res[a]['csi']) - norm_csi(res[b]['csi']))}; only in {b}: {sorted(norm_csi(res[b]['csi']) - norm_csi(res[a]['csi']))}")
            for shot in ("boot", "reply", "second"):
                print(f"  {a} vs {b} style set @{shot} equal: {res[a]['styles'][shot] == res[b]['styles'][shot]}; only {a}: {sorted(map(str, res[a]['styles'][shot] - res[b]['styles'][shot]))[:4]} only {b}: {sorted(map(str, res[b]['styles'][shot] - res[a]['styles'][shot]))[:4]}")
    server.shutdown()


PANE_SCRIPT = r'''
import os, sys, termios, tty, select, time
fd = os.open("/dev/tty", os.O_RDWR)
tty.setraw(fd)
log = open(sys.argv[1], "a", buffering=1)
def ask(q, secs=1.0):
    os.write(fd, q)
    got = b""; end = time.time() + secs
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r: got += os.read(fd, 4096)
    log.write(f"ASK {q!r} -> {got!r}\n")
ask(b"\x1b[?u")
ask(b"\x1b[c")
os.write(fd, b"\x1b[>1u")
log.write("PUSHED\n")
ask(b"\x1b[?u")
log.write("READY\n")
end = time.time() + float(sys.argv[2])
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        b = os.read(fd, 4096)
        log.write(f"T+{time.time():.2f} GOT {b!r}\n")
        if b.startswith(b"MARK"): pass
os.write(fd, b"\x1b[<u")
log.write("POPPED\n")
ask(b"\x1b[?u")
end = time.time() + float(sys.argv[3])
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        log.write(f"T+{time.time():.2f} GOT-AFTER-POP {os.read(fd, 4096)!r}\n")
log.write("DONE\n")
'''


def herdr():
    spec = importlib.util.spec_from_file_location("native_host", os.path.join(HERE, "native-host.py"))
    nh = importlib.util.module_from_spec(spec)
    sys.argv = [sys.argv[0]]
    spec.loader.exec_module(nh)
    import ptyclient
    script = os.path.join(nh.WORK, "kitty-pane.py")
    with open(script, "w") as f:
        f.write(PANE_SCRIPT)
    log = os.path.join(nh.WORK, "pane.log")
    h = nh.session("kitty")
    client = None
    try:
        p = "w1:p1"
        time.sleep(1.0)
        pid, fd = ptyclient.spawn(["herdr", "--session", h.name], 120, 40, env={"HOME": h.env["HOME"]})
        client = (pid, fd)
        ptyclient_boot = ptyclient.drain(fd, 1.0)
        if os.environ.get("KCK_OUTER") == "kitty":
            os.write(fd, b"\x1b[?7u\x1b[?1016;2$y\x1b[?997;1n\x1b[?62;22c")
        ptyclient_boot += ptyclient.drain(fd, 1.0)
        out = bytearray()
        stop = threading.Event()

        def reader():
            while not stop.is_set():
                r, _, _ = select.select([fd], [], [], 0.05)
                if r:
                    try:
                        out.extend(os.read(fd, 65536))
                    except OSError:
                        return
        threading.Thread(target=reader, daemon=True).start()
        h.type(p, f" unset HISTFILE; exec {sys.executable} -I {script} {log} 70 6")
        t0 = time.time()
        while time.time() - t0 < 15:
            if os.path.exists(log) and "READY" in open(log).read():
                break
            time.sleep(0.1)
        time.sleep(1.0)
        seen = bytes(out)
        print("client output after push, kitty seqs:", re.findall(rb"\x1b\[[<>=?][0-9;]*u", seen))
        print("client boot queries/modes:", sorted(set(re.findall(rb"\x1b\[[?>=<]?[0-9;]*[$ ]?[a-zA-Z]|\x1b\][0-9]+;\?[^\x07\x1b]*", ptyclient_boot)))[:40])

        def mark(name):
            with open(log, "a") as f:
                f.write(f"--- {name}\n")

        for keys in (["Escape"], ["Enter"], ["shift+Enter"], ["ctrl+c"], ["alt+Enter"], ["Tab"], ["Backspace"], ["a"], ["ctrl+a"]):
            mark(f"api pane.send_keys {keys}")
            r = h("pane.send_keys", pane_id=p, keys=keys)
            if "error" in r:
                mark(f"error {r['error']}")
            time.sleep(0.6)
        kitty_outer = os.environ.get("KCK_OUTER") == "kitty"
        legacy = (("text x", b"x"), ("cr", b"\r"), ("del", b"\x7f"), ("ctrl-c", b"\x03"), ("alt-enter", b"\x1b\r"), ("tab", b"\t"))
        kitty = (("kitty cr", b"\x1b[13u"), ("kitty shift-enter", b"\x1b[13;2u"), ("kitty ctrl-c", b"\x1b[99;5u"),
                 ("kitty del", b"\x1b[127u"), ("kitty esc", b"\x1b[27u"), ("kitty x", b"\x1b[120u"))
        time.sleep(float(os.environ.get("KCK_GAP", "0")))
        for rnd, (name, b) in [(r, k) for r in (1, 2) for k in (kitty if kitty_outer else legacy) + (("esc last", b"\x1b"),)]:
            mark(f"client pty round {rnd} {name} {b!r}")
            os.write(fd, b)
            time.sleep(1.0)
        stop.set()
        seen = bytes(out)
        print("client output total kitty seqs:", re.findall(rb"\x1b\[[<>=?][0-9;]*u", seen))
        sc = pyte.Screen(120, 40)
        pyte.ByteStream(sc).feed(PYTE_UNSAFE.sub(b"", ptyclient_boot + seen))
        print("client screen:\n" + "\n".join(l.rstrip() for l in sc.display if l.strip()))
        t0 = time.time()
        while time.time() - t0 < 60:
            if "DONE" in open(log).read():
                break
            time.sleep(0.2)
        print(open(log).read())
    finally:
        if client:
            try:
                os.kill(client[0], signal.SIGKILL)
                os.waitpid(client[0], 0)
            except OSError:
                pass
        h.stop()
        shutil.rmtree(nh.WORK, ignore_errors=True)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "claude"
    if mode == "herdr":
        herdr()
    else:
        claude(sys.argv[2:])
