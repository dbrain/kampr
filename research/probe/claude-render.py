#!/usr/bin/env python3
"""What Claude Code writes to a pty it owns outright, in both of its renderers: the screen it takes,
how a streamed reply arrives, whether it wraps its own text, and what a resize makes it send.

    python3 research/probe/claude-render.py [fullscreen|default ...]

This is the view a native holder (docs/17) would have, so no herdr is involved. Claude runs under
a throwaway CLAUDE_CONFIG_DIR against a mock Messages endpoint served from this process, which
streams one scripted reply a word at a time, so no account or network is touched.
"""

import fcntl
import http.server
import json
import os
import pty
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

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import vt

COLS, ROWS = 100, 30
KEY = "sk-ant-api03-probe-" + "0" * 40

PARA = ("PARA1START a terminal emulator sits between a program writing bytes and a person reading "
        "glyphs, and almost everything hard about one lives in that gap, because the program writes "
        "a stream with no idea how wide the screen is while the screen has a fixed grid and a cursor "
        "that the stream moves about, so every question of wrapping is settled by whoever knows the "
        "width at the moment the bytes land PARA1END")
REPLY = "\n\n".join([
    "## Heading for the probe",
    PARA,
    "\n".join(f"- bullet {i} with a few words so the list has some body to it" for i in range(1, 9)),
    "```\n" + "\n".join(f"code line {i}: {'x' * 30}" for i in range(1, 9)) + "\n```",
    PARA.replace("PARA1", "PARA2"),
    "REPLYEND",
])


def sse(event, data):
    return f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()


class Mock(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        if self.path.split("?")[0].endswith("count_tokens"):
            return self.json({"input_tokens": 10})
        prompt = json.dumps([m for m in body.get("messages", []) if m.get("role") == "user"][-1:])
        if os.environ.get("KCR_DEBUG"):
            print("    request", self.path, body.get("stream"), len(body.get("messages", [])), prompt[:300], file=sys.stderr)
        text = REPLY if "LONGANSWER" in prompt else "Short answer."
        if not body.get("stream"):
            return self.json({"id": "msg_p", "type": "message", "role": "assistant", "model": body.get("model"),
                              "content": [{"type": "text", "text": "Probe title"}], "stop_reason": "end_turn",
                              "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 3}})
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        w = self.wfile
        w.write(sse("message_start", {"type": "message_start", "message": {
            "id": "msg_p", "type": "message", "role": "assistant", "model": body.get("model"), "content": [],
            "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}}))
        w.write(sse("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}))
        for word in re.split(r"(?<= )", text):
            w.write(sse("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": word}}))
            w.flush()
            time.sleep(0.015)
        w.write(sse("content_block_stop", {"type": "content_block_stop", "index": 0}))
        w.write(sse("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 50}}))
        w.write(sse("message_stop", {"type": "message_stop"}))
        w.flush()

    def json(self, obj):
        data = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class Claude:
    def __init__(self, tui, port, work):
        cfg = os.path.join(work, "cfg")
        cwd = os.path.join(work, "proj")
        os.makedirs(cfg)
        os.makedirs(cwd)
        with open(os.path.join(cfg, ".claude.json"), "w") as f:
            json.dump({"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 5,
                       "customApiKeyResponses": {"approved": [KEY[-20:]], "rejected": []},
                       "projects": {cwd: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}}, f)
        with open(os.path.join(cfg, "settings.json"), "w") as f:
            json.dump({"tui": tui}, f)
        env = {k: v for k, v in os.environ.items() if not k.startswith(("CLAUDE", "HERDR_", "ANTHROPIC"))}
        env |= {"CLAUDE_CONFIG_DIR": cfg, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{port}", "ANTHROPIC_API_KEY": KEY,
                "DISABLE_TELEMETRY": "1", "DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
                "TERM": "xterm-256color", "COLORTERM": "truecolor"}
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execvpe("claude", ["claude"], env)
        self.resize(COLS, ROWS)
        self.log = []

    def resize(self, cols, rows):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def pump(self, secs, until=None):
        end = time.time() + secs
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    return
                self.log.append((time.time(), chunk))
                if until and until in self.since(0)[-4000:]:
                    return

    def since(self, i):
        return b"".join(c for _, c in self.log[i:])

    def type(self, text):
        for ch in text:
            os.write(self.fd, ch.encode())
            time.sleep(0.01)

    def close(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass


def counts(b):
    keys = {"?1049h": rb"\x1b\[\?1049h", "?2026h": rb"\x1b\[\?2026h", "2J": rb"\x1b\[2J", "3J": rb"\x1b\[3J",
            "home": rb"\x1b\[H|\x1b\[1;1H", "?1000h": rb"\x1b\[\?1000h", "scroll-region": rb"\x1b\[\d+;\d+r"}
    return {k: len(re.findall(v, b)) for k, v in keys.items()}


STRIP = re.compile(rb"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78DEM]")


def wrapping(b, start, end):
    s = STRIP.sub(lambda m: b"\x00" if m.group().endswith((b"H", b"B", b"E", b"f", b"d", b"G")) else b"", b)
    i = s.rfind(start)
    j = s.find(end, i)
    if i < 0 or j < 0:
        return "paragraph not found whole in the byte stream"
    seg = s[i:j + len(end)]
    runs = [r for r in re.split(rb"[\r\n\x00]+", seg) if r.strip()]
    return f"{len(runs)} runs between markers, longest {max(len(r.decode('utf8', 'replace')) for r in runs)} cols (pty {COLS})"


def screen(b, cols=COLS, rows=ROWS):
    sc = vt.Screen(cols, rows)
    vt.feed(sc, b.decode("utf8", "replace"))
    return [l for l in sc.text().splitlines() if l.strip()]


def save(name, data):
    with open(os.path.join(os.environ.get("KCR_OUT", tempfile.gettempdir()), name), "wb") as f:
        f.write(data)


def run(tui, port):
    work = tempfile.mkdtemp(prefix="kcr-", dir=os.environ.get("XDG_RUNTIME_DIR"))
    c = Claude(tui, port, work)
    try:
        c.pump(12, until=b"\xe2\x9d\xaf")
        c.pump(2)
        boot = c.since(0)
        print(f"=== tui={tui}: claude up after {len(boot)} bytes, {counts(boot)}")
        for line in screen(boot)[-6:]:
            print("   |", line[:COLS])
        mark = len(c.log)
        c.type("LONGANSWER please")
        c.pump(1)
        c.type("\r")
        t0 = time.time()
        c.pump(40, until=b"REPLYEND")
        c.pump(3)
        stream = c.since(mark)
        chunks = len(c.log) - mark
        print(f"  streamed reply: {len(stream)} bytes in {chunks} reads over {time.time() - t0:.1f}s for {len(REPLY)} chars of text; {counts(stream)}")
        print("  wrapping:", wrapping(stream, b"PARA1START", b"PARA1END"))
        save(f"claude-{tui}-turn.bin", c.since(0))
        for cols in (60, 100):
            mark = len(c.log)
            c.resize(cols, ROWS)
            c.pump(4)
            b = c.since(mark)
            print(f"  resize to {cols}: {len(b)} bytes, {counts(b)}, PARA1START x{b.count(b'PARA1START')}, "
                  f"heading x{b.count(b'Heading for the probe')}")
        mark = len(c.log)
        c.type("SHORT one")
        c.pump(1)
        c.type("\r")
        c.pump(10, until=b"Short answer")
        c.pump(2)
        b = c.since(mark)
        print(f"  a second short turn: {len(b)} bytes, {counts(b)}, PARA1START x{b.count(b'PARA1START')}")
        whole = c.since(0)
        print(f"  whole run: PARA1START x{whole.count(b'PARA1START')} over {len(whole)} bytes")
        save(f"claude-{tui}.bin", whole)
    finally:
        c.close()
        shutil.rmtree(work, ignore_errors=True)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mock)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    for tui in sys.argv[1:] or ["fullscreen", "default"]:
        run(tui, server.server_address[1])
    server.shutdown()


if __name__ == "__main__":
    main()
