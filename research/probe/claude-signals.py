#!/usr/bin/env python3
"""What Claude Code tells a terminal it does not know is watching, with no hooks and no statusline:
window titles, notifications, progress, OSC 7501 program status, OSC 133, and OpenTelemetry, set
against the session marker and the screen for one scripted turn that streams, asks permission for
a tool, asks a question, finishes, compacts and fails.

    python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte
    /tmp/v/bin/python research/probe/claude-signals.py matrix [variant ...]   # run, then report
    /tmp/v/bin/python research/probe/claude-signals.py run <variant> <out>    # one run
    /tmp/v/bin/python research/probe/claude-signals.py report <out>           # compare runs

Every run is its own claude under a throwaway CLAUDE_CONFIG_DIR on a pty this script owns, talking
to a mock Messages endpoint served from this process, with an OTLP/http-json collector beside it.
A variant fixes the environment, the settings and which terminal queries get answered, and how.
Nothing touches the network, the operator's config, or herdr.
"""

import base64
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
import subprocess
import sys
import tempfile
import termios
import threading
import time

import pyte

COLS, ROWS = 110, 34
KEY = "sk-ant-api03-probe-" + "0" * 40
QUESTION = "Which probe colour should win"
KAMPR_XTV = "kampr 0.1.118"

TERMINAL_ENV = ("CLAUDE", "HERDR_", "ANTHROPIC", "OTEL_", "TERM_PROGRAM", "KITTY_", "WEZTERM", "ITERM", "GHOSTTY",
                "VTE_VERSION", "TMUX", "STY", "WT_SESSION", "LC_TERMINAL", "KONSOLE", "ALACRITTY", "ZED_TERM",
                "TERMINAL_EMULATOR", "VSCODE", "DISABLE_", "KAMPR_")

BASE = {"answers": {"da1": True, "xtversion": KAMPR_XTV}, "env": {}, "settings": {}, "config": {}, "otel": False,
        "quiet": True}


def v(**kw):
    out = json.loads(json.dumps(BASE))
    for k, val in kw.items():
        if isinstance(val, dict) and k != "answers":
            out[k] |= val
        else:
            out[k] = val
    return out


VARIANTS = {
    "kampr": v(),
    "silent": v(answers={}),
    "da1-only": v(answers={"da1": True}),
    "osc7501": v(answers={"da1": True, "xtversion": KAMPR_XTV, "osc7501": True}),
    "osc7501-only": v(answers={"da1": True, "osc7501": True}),
    "xtv-ghostty": v(answers={"da1": True, "xtversion": "ghostty 1.2.3"}),
    "xtv-iterm": v(answers={"da1": True, "xtversion": "iTerm2 3.6.6"}),
    "xtv-kitty": v(answers={"da1": True, "xtversion": "kitty(0.42.1)"}),
    "xtv-wezterm": v(answers={"da1": True, "xtversion": "WezTerm 20240203-110809-5046fc22"}),
    "xtv-xtermjs": v(answers={"da1": True, "xtversion": "xterm.js(5.5.0)"}),
    "sync2026": v(answers={"da1": True, "xtversion": KAMPR_XTV, "decrqm": {"2026": 2}}),
    "tp-ghostty": v(env={"TERM_PROGRAM": "ghostty", "TERM_PROGRAM_VERSION": "1.2.3"}),
    "tp-iterm": v(env={"TERM_PROGRAM": "iTerm.app", "TERM_PROGRAM_VERSION": "3.6.6"}),
    "tp-kitty": v(env={"TERM_PROGRAM": "kitty"}),
    "tp-wezterm": v(env={"TERM_PROGRAM": "WezTerm"}),
    "ch-iterm2": v(config={"preferredNotifChannel": "iterm2"}),
    "ch-bell": v(config={"preferredNotifChannel": "terminal_bell"}),
    "ch-iterm2bell": v(config={"preferredNotifChannel": "iterm2_with_bell"}),
    "ch-kitty": v(config={"preferredNotifChannel": "kitty"}),
    "ch-ghostty": v(config={"preferredNotifChannel": "ghostty"}),
    "ch-disabled": v(config={"preferredNotifChannel": "notifications_disabled"}),
    "progress-off": v(env={"TERM_PROGRAM": "ghostty", "TERM_PROGRAM_VERSION": "1.2.3"},
                      config={"terminalProgressBarEnabled": False}),
    "loud": v(quiet=False),
    "otel": v(otel=True, quiet=False),
    "otel-quiet": v(otel=True),
    "otel-operator": v(otel="operator", quiet=False),
}


def sse(event, data):
    return f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()


def last_user(body):
    users = [m for m in body.get("messages", []) if m.get("role") == "user"]
    return users[-1] if users else {}


def results_in(msg):
    c = msg.get("content")
    return [b.get("tool_use_id") for b in c if isinstance(b, dict) and b.get("type") == "tool_result"] if isinstance(c, list) else []


class Mock(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    requests = None

    def log_message(self, *_):
        pass

    def do_GET(self):
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        if self.path.split("?")[0].endswith("count_tokens"):
            return self.json({"input_tokens": 10})
        user = last_user(body)
        done = results_in(user)
        text = json.dumps(user)
        system = json.dumps(body.get("system", ""))[:200]
        if self.requests is not None:
            self.requests.append({"t": time.time(), "stream": body.get("stream"), "results": done,
                                  "user": text[:160], "system": system[:120]})
        if not body.get("stream"):
            return self.json({"id": "msg_p", "type": "message", "role": "assistant", "model": body.get("model"),
                              "content": [{"type": "text", "text": '{"title": "Probe topic words"}'}],
                              "stop_reason": "end_turn", "stop_sequence": None,
                              "usage": {"input_tokens": 10, "output_tokens": 3}})
        if text.startswith('{"role": "user", "content": [{"type": "text", "text": "<session>'):
            return self.stream('{"title": "Probe topic words"}', None)
        if "FAILME" in text and "SLOWTURN" not in text and not done:
            return self.json({"type": "error", "error": {"type": "invalid_request_error", "message": "probe failure"}},
                             status=400)
        if "toolu_ask01" in done:
            return self.stream("All answered. " + "Closing words for the probe. " * 6 + "ALLDONE", None)
        if "toolu_perm01" in done:
            return self.stream("The file is made. Now a question for you. ", ("toolu_ask01", "AskUserQuestion", {
                "questions": [{"question": QUESTION + "?", "header": "Colour", "multiSelect": False,
                               "options": [{"label": "Teal", "description": "A blue green."},
                                           {"label": "Amber", "description": "A yellow orange."}]}]}))
        if "SLOWTURN" in text and not done:
            return self.stream("Slow words. " * 120, None)
        if "SIGNALS" in text:
            return self.stream("Let me make a file first. " + "Thinking out loud for a while. " * 8,
                               ("toolu_perm01", "Bash", {"command": "touch probe-made-this.txt",
                                                          "description": "Create a probe file"}))
        if "compact" in system.lower() or "summar" in text.lower():
            return self.stream("<summary>Probe summary of the conversation so far.</summary>", None)
        return self.stream("Short answer.", None)

    def stream(self, words, tool):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        w = self.wfile
        w.write(sse("message_start", {"type": "message_start", "message": {
            "id": "msg_p", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5", "content": [],
            "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}}))
        w.write(sse("content_block_start", {"type": "content_block_start", "index": 0,
                                            "content_block": {"type": "text", "text": ""}}))
        for word in re.split(r"(?<= )", words):
            w.write(sse("content_block_delta", {"type": "content_block_delta", "index": 0,
                                                "delta": {"type": "text_delta", "text": word}}))
            w.flush()
            time.sleep(0.04)
        w.write(sse("content_block_stop", {"type": "content_block_stop", "index": 0}))
        if tool:
            tid, name, inp = tool
            w.write(sse("content_block_start", {"type": "content_block_start", "index": 1,
                                                "content_block": {"type": "tool_use", "id": tid, "name": name, "input": {}}}))
            w.write(sse("content_block_delta", {"type": "content_block_delta", "index": 1,
                                                "delta": {"type": "input_json_delta", "partial_json": json.dumps(inp)}}))
            w.write(sse("content_block_stop", {"type": "content_block_stop", "index": 1}))
        w.write(sse("message_delta", {"type": "message_delta",
                                      "delta": {"stop_reason": "tool_use" if tool else "end_turn", "stop_sequence": None},
                                      "usage": {"output_tokens": 50}}))
        w.write(sse("message_stop", {"type": "message_stop"}))
        w.flush()

    def json(self, obj, status=200):
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class Collector(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    sink = None
    name = "kampr"

    def log_message(self, *_):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        try:
            body = json.loads(raw)
        except ValueError:
            body = {"undecodable": len(raw), "type": self.headers.get("content-type")}
        self.sink.append({"t": time.time(), "path": self.path, "collector": self.name, "body": body})
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", "2")
        self.end_headers()
        self.wfile.write(b"{}")


def serve(handler, **attrs):
    cls = type(handler.__name__, (handler,), attrs)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), cls)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


# pyte prints an APC's body as text where every real terminal drops it (Claude's kitty graphics query).
APC = re.compile(rb"\x1b_[^\x1b]*\x1b\\")

SCREEN_FLAGS = [("spinner", "esc to interrupt"), ("permission", "Do you want"), ("question", "Enter to select"),
                ("error", "API Error")]

QUERIES = [
    ("da1", re.compile(rb"\x1b\[0?c")),
    ("da2", re.compile(rb"\x1b\[>0?c")),
    ("xtversion", re.compile(rb"\x1b\[>0?q")),
    ("kitty-kbd", re.compile(rb"\x1b\[\?u")),
    ("osc7501", re.compile(rb"\x1b\]7501;\?(?:\x07|\x1b\\)")),
    ("decrqm", re.compile(rb"\x1b\[\?(\d+)\$p")),
    ("cellsize", re.compile(rb"\x1b\[16t")),
    ("cpr", re.compile(rb"\x1b\[\??6n")),
    ("kitty-gfx", re.compile(rb"\x1b_G[^\x1b]*\x1b\\")),
    ("osc-colour", re.compile(rb"\x1b\]1[01];\?(?:\x07|\x1b\\)")),
]


def answer(kind, m, answers, screen):
    if kind == "da1" and answers.get("da1"):
        return b"\x1b[?62;22c"
    if kind == "xtversion" and answers.get("xtversion"):
        return b"\x1bP>|" + answers["xtversion"].encode() + b"\x1b\\"
    if kind == "osc7501" and answers.get("osc7501"):
        return b"\x1b]7501;?\x1b\\"
    if kind == "decrqm":
        mode = m.group(1).decode()
        val = answers.get("decrqm", {}).get(mode)
        if val is None and answers.get("xtversion") == KAMPR_XTV:
            val = 0
        return None if val is None else f"\x1b[?{mode};{val}$y".encode()
    if kind == "cpr" and answers.get("da1"):
        return f"\x1b[{screen.cursor.y + 1};{screen.cursor.x + 1}R".encode()
    return None


class Run:
    def __init__(self, name, spec, out, mock_port):
        self.name, self.spec, self.out = name, spec, out
        self.work = tempfile.mkdtemp(prefix="kcs-", dir=os.environ.get("XDG_RUNTIME_DIR"))
        self.cfg = os.path.join(self.work, "cfg")
        self.cwd = os.path.join(self.work, "proj")
        os.makedirs(self.cfg)
        os.makedirs(self.cwd)
        self.log, self.queries, self.seen, self.snaps, self.marker, self.agents = [], [], {}, {}, [], []
        self.otel = []
        self.collectors = []
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.lock = threading.Lock()
        self.watch = ["\u276f", "Do you want", QUESTION, "ALLDONE", "Compacted", "API Error"]
        self.flags = []
        self.stop = False
        self.t0 = time.time()
        config = {"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 5,
                  "messageIdleNotifThresholdMs": 3000,
                  "customApiKeyResponses": {"approved": [KEY[-20:]], "rejected": []},
                  "projects": {self.cwd: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}}
        config |= spec["config"]
        settings = {"tui": "fullscreen", "permissions": {"disableAutoMode": "disable"}} | spec["settings"]
        env = {k: val for k, val in os.environ.items() if not k.startswith(TERMINAL_ENV)}
        env |= {"CLAUDE_CONFIG_DIR": self.cfg, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{mock_port}",
                "ANTHROPIC_API_KEY": KEY, "DISABLE_AUTOUPDATER": "1", "TERM": "xterm-256color", "COLORTERM": "truecolor"}
        if spec["otel"]:
            kampr = serve(Collector, sink=self.otel, name="kampr")
            self.collectors.append(kampr)
            env |= {"CLAUDE_CODE_ENABLE_TELEMETRY": "1", "OTEL_LOGS_EXPORTER": "otlp", "OTEL_METRICS_EXPORTER": "otlp",
                    "OTEL_TRACES_EXPORTER": "otlp", "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA": "1",
                    "OTEL_EXPORTER_OTLP_PROTOCOL": "http/json",
                    "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{kampr.server_address[1]}",
                    "OTEL_LOGS_EXPORT_INTERVAL": "100", "OTEL_METRIC_EXPORT_INTERVAL": "1000",
                    "OTEL_TRACES_EXPORT_INTERVAL": "100"}
            if spec["otel"] == "operator":
                operator = serve(Collector, sink=self.otel, name="operator")
                self.collectors.append(operator)
                settings["env"] = {"CLAUDE_CODE_ENABLE_TELEMETRY": "1", "OTEL_LOGS_EXPORTER": "otlp",
                                   "OTEL_EXPORTER_OTLP_PROTOCOL": "http/json",
                                   "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{operator.server_address[1]}"}
        if spec["quiet"]:
            env |= {"DISABLE_TELEMETRY": "1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"}
        env |= spec["env"]
        with open(os.path.join(self.cfg, ".claude.json"), "w") as f:
            json.dump(config, f)
        with open(os.path.join(self.cfg, "settings.json"), "w") as f:
            json.dump(settings, f)
        self.env = env
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(self.cwd)
            os.execvpe("claude", ["claude"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        threading.Thread(target=self.reader, daemon=True).start()
        threading.Thread(target=self.marker_poll, daemon=True).start()

    def rel(self, t=None):
        return round((t or time.time()) - self.t0, 3)

    def reader(self):
        tail = b""
        while not self.stop:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if not r:
                continue
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            now = time.time()
            with self.lock:
                self.log.append((now, chunk))
                try:
                    self.stream.feed(APC.sub(b"", chunk))
                except Exception as e:
                    self.queries.append({"t": self.rel(now), "kind": "pyte-error", "err": repr(e)})
                display = "\n".join(self.screen.display)
                for key in self.watch:
                    if key not in self.seen and key in display:
                        self.seen[key] = self.rel(now)
                flags = tuple(k for k, probe in SCREEN_FLAGS if probe in display)
                if not self.flags or self.flags[-1][1] != flags:
                    self.flags.append((self.rel(now), flags))
                scan = tail + chunk
                # In stream order, as a terminal would: Claude settles every query still unanswered
                # when the DA1 reply lands, so an answer behind it counts as no answer.
                found = sorted((m.start(), kind, m) for kind, rx in QUERIES for m in rx.finditer(scan)
                               if m.end() > len(tail))
                for _, kind, m in found:
                    reply = answer(kind, m, self.spec["answers"], self.screen)
                    self.queries.append({"t": self.rel(now), "kind": kind, "raw": m.group().decode("latin1"),
                                         "answered": reply.decode("latin1") if reply else None})
                    if reply:
                        os.write(self.fd, reply)
                tail = scan[-256:]

    def marker_poll(self):
        last = None
        d = os.path.join(self.cfg, "sessions")
        while not self.stop:
            state = None
            try:
                for n in os.listdir(d):
                    if n.endswith(".json"):
                        j = json.load(open(os.path.join(d, n)))
                        state = (j.get("status"), j.get("waitingFor"))
            except (OSError, ValueError):
                pass
            if state != last:
                self.marker.append({"t": self.rel(), "status": state[0] if state else None,
                                    "waitingFor": state[1] if state else None, "exists": state is not None})
                last = state
            time.sleep(0.02)

    def until(self, key, secs):
        if key not in self.watch:
            self.watch.append(key)
            with self.lock:
                if key in "\n".join(self.screen.display):
                    self.seen[key] = self.rel()
        end = time.time() + secs
        while time.time() < end:
            if key in self.seen:
                return True
            time.sleep(0.02)
        return False

    def type(self, text):
        for ch in text:
            os.write(self.fd, ch.encode())
            time.sleep(0.015)

    def snap(self, name):
        with self.lock:
            text = [l.rstrip() for l in self.screen.display]
            style = []
            for y in range(ROWS):
                row = self.screen.buffer[y]
                style.append("".join(f"{row[x].fg[:3]}{row[x].bg[:3]}{int(row[x].bold)}{int(row[x].reverse)}"
                                     for x in range(COLS)))
        self.snaps[name] = {"t": self.rel(), "text": text, "style": style}

    def agents_json(self, label):
        t = time.time()
        try:
            out = subprocess.run(["claude", "agents", "--json"], env=self.env, capture_output=True, timeout=20,
                                 text=True).stdout
        except subprocess.TimeoutExpired:
            out = "timeout"
        self.agents.append({"label": label, "t": self.rel(t), "took": round(time.time() - t, 3), "out": out[:2000]})

    def phase(self, name, at=None):
        self.seen[f"@{name}"] = self.rel(at)

    def script(self):
        ok = self.until("❯", 25)
        time.sleep(2.5)
        self.snap("boot")
        self.phase("boot-ready")
        if not ok:
            return
        self.type("SIGNALS go")
        time.sleep(0.6)
        self.phase("submit")
        os.write(self.fd, b"\r")
        if self.until("Do you want", 30):
            time.sleep(0.8)
            self.snap("permission")
            if self.name in ("kampr", "otel"):
                self.agents_json("permission")
            time.sleep(7)
            self.phase("approve")
            os.write(self.fd, b"\r")
        if self.until(QUESTION, 30):
            time.sleep(0.8)
            self.snap("question")
            if self.name in ("kampr", "otel"):
                self.agents_json("question")
            time.sleep(7)
            self.phase("answer")
            os.write(self.fd, b"\r")
            for _ in range(3):
                if self.until("ALLDONE", 2.5):
                    break
                self.phase("answer-again")
                os.write(self.fd, b"\r")
        self.until("ALLDONE", 30)
        time.sleep(1.5)
        self.snap("done")
        if self.name in ("kampr", "otel"):
            self.agents_json("done")
        time.sleep(6)
        self.snap("idle")
        self.type("/compact")
        time.sleep(0.8)
        self.phase("compact")
        os.write(self.fd, b"\r")
        self.until("ompact", 3)
        time.sleep(8)
        self.snap("compacted")
        self.type("FAILME now")
        time.sleep(0.6)
        self.phase("fail")
        os.write(self.fd, b"\r")
        time.sleep(6)
        self.snap("failed")
        self.type("SLOWTURN please")
        time.sleep(0.6)
        self.phase("slow")
        os.write(self.fd, b"\r")
        time.sleep(2.5)
        self.phase("escape")
        os.write(self.fd, b"\x1b")
        time.sleep(5)
        self.snap("interrupted")
        self.type("/exit")
        time.sleep(0.8)
        self.phase("exit")
        os.write(self.fd, b"\r")
        time.sleep(4)

    def files(self):
        out = []
        for base, _, names in os.walk(self.cfg):
            for n in names:
                p = os.path.join(base, n)
                out.append(os.path.relpath(p, self.cfg))
        return sorted(out)

    def close(self):
        try:
            self.files_seen = self.files()
            self.cc_socks = sorted(os.listdir(f"/run/user/{os.getuid()}/cc-socks")) if os.path.isdir(
                f"/run/user/{os.getuid()}/cc-socks") else []
        finally:
            self.stop = True
            try:
                os.kill(self.pid, signal.SIGTERM)
                time.sleep(1.0)
                os.kill(self.pid, signal.SIGKILL)
            except OSError:
                pass
            try:
                os.waitpid(self.pid, 0)
            except OSError:
                pass
            for c in self.collectors:
                c.shutdown()
            time.sleep(0.3)
            shutil.rmtree(self.work, ignore_errors=True)

    def save(self, requests):
        whole = b"".join(c for _, c in self.log)
        with open(os.path.join(self.out, f"{self.name}.bin"), "wb") as f:
            f.write(whole)
        offsets, pos = [], 0
        for t, c in self.log:
            pos += len(c)
            offsets.append((pos, self.rel(t)))
        res = {"name": self.name, "spec": self.spec, "phases": self.seen, "flags": self.flags, "queries": self.queries,
               "marker": self.marker, "agents": self.agents, "snaps": self.snaps, "offsets": offsets,
               "otel": self.otel, "files": getattr(self, "files_seen", []), "requests": requests,
               "pid": self.pid, "cc_socks_had_pid": f"{self.pid}.sock" in getattr(self, "cc_socks", [])}
        with open(os.path.join(self.out, f"{self.name}.json"), "w") as f:
            json.dump(res, f)


def run_one(name, out):
    requests = []
    mock = serve(Mock, requests=requests)
    r = Run(name, VARIANTS[name], out, mock.server_address[1])
    try:
        r.script()
    finally:
        r.close()
        for req in requests:
            req["t"] = r.rel(req["t"])
        r.save(requests)
        mock.shutdown()


OSC = re.compile(rb"\x1b\](\d+);?([^\x07\x1b]*)(?:\x07|\x1b\\)")
MODES = re.compile(rb"\x1b\[\?([\d;]+)([hl])")
KITTY_PUSH = re.compile(rb"\x1b\[>(\d+)u")


def at(offsets, pos):
    for end, t in offsets:
        if pos < end:
            return t
    return offsets[-1][1] if offsets else None


def oscs(res, raw):
    out = []
    for m in OSC.finditer(raw):
        code = int(m.group(1))
        if code in (8, 10, 11, 12, 52, 104, 110, 111, 112):
            continue
        data = m.group(2).decode("utf8", "replace")
        if code == 7501:
            parts = dict(p.split("=", 1) for p in data.split(":") if "=" in p)
            for k in ("title", "msg"):
                if k in parts:
                    try:
                        parts[k] = base64.b64decode(parts[k]).decode("utf8", "replace")
                    except ValueError:
                        pass
            data = parts
        out.append((at(res["offsets"], m.end()), code, data))
    return out


def bells(raw):
    stripped = OSC.sub(lambda m: b"\x00" * len(m.group()), raw)
    stripped = re.sub(rb"\x1bP.*?\x1b\\", lambda m: b"\x00" * len(m.group()), stripped, flags=re.S)
    return [i for i, b in enumerate(stripped) if b == 7]


MASK = re.compile(r"\d+|[⠀-⣿·✢✳✶✻✽∗◐-◓●*]|\b[A-Z][\w\u00c0-\u017f]+(ing|ed)\b…?")


def masked(lines):
    return [MASK.sub("#", re.sub(r"kcs-[a-z0-9_]+", "kcs-X", l)) for l in lines]


def diff_snaps(a, b):
    out = {}
    for k in a["snaps"]:
        if k not in b["snaps"]:
            out[k] = "missing"
            continue
        ta, tb = masked(a["snaps"][k]["text"]), masked(b["snaps"][k]["text"])
        rows = [i for i, (x, y) in enumerate(zip(ta, tb)) if x != y]
        srows = [i for i, (x, y) in enumerate(zip(a["snaps"][k]["style"], b["snaps"][k]["style"]))
                 if x != y and i not in rows and ta[i] == tb[i]]
        if rows or srows:
            out[k] = {"text_rows": [(i, ta[i], tb[i]) for i in rows[:6]],
                      "style_only_rows": [(i, ta[i][:60]) for i in srows[:10]]}
    return out


def report(out):
    runs = {}
    for n in sorted(os.listdir(out)):
        if n.endswith(".json"):
            runs[n[:-5]] = json.load(open(os.path.join(out, n)))
    base = runs.get("kampr")
    for name, res in runs.items():
        raw = open(os.path.join(out, f"{name}.bin"), "rb").read()
        print(f"\n=== {name}: {len(raw)} bytes, phases {res['phases']}")
        print("  queries:", [(q["t"], q["kind"], q.get("raw", "")[2:12], q.get("answered") and q["answered"][:30])
                             for q in res["queries"]])
        modes = {}
        for m in MODES.finditer(raw):
            for p in m.group(1).split(b";"):
                modes.setdefault(p.decode(), set()).add(m.group(2).decode())
        print("  private modes:", {k: "".join(sorted(v)) for k, v in sorted(modes.items(), key=lambda x: int(x[0]))})
        print("  kitty push:", sorted(set(m.group(1).decode() for m in KITTY_PUSH.finditer(raw))))
        last = None
        for t, code, data in oscs(res, raw):
            key = (code, json.dumps(data, sort_keys=True))
            if key == last:
                continue
            last = key
            print(f"  {t:8.3f} OSC {code} {data if code != 0 or len(str(data)) < 90 else str(data)[:90]}")
        bel = bells(raw)
        print("  BEL at:", sorted(set(at(res["offsets"], i + 1) for i in bel)))
        print("  screen:", res.get("flags"))
        print("  marker:", [(m["t"], m["status"], m["waitingFor"]) for m in res["marker"]])
        for a in res["agents"]:
            print(f"  agents --json @{a['label']} t={a['t']} took {a['took']}s: {a['out'].strip()[:300]}")
        if res["otel"]:
            for e in res["otel"]:
                b = e["body"]
                for rl in b.get("resourceLogs", []):
                    for sl in rl.get("scopeLogs", []):
                        for lr in sl.get("logRecords", []):
                            attrs = {a["key"]: list(a["value"].values())[0] for a in lr.get("attributes", [])}
                            ts = int(lr.get("timeUnixNano", 0)) / 1e9
                            print(f"  {round(e['t'], 3):>14} [{e['collector']}] log {lr.get('body', {}).get('stringValue')} "
                                  f"{ {k: attrs[k] for k in attrs if k in ('event.name', 'session.id', 'prompt.id', 'tool_name', 'decision', 'source', 'success', 'event.sequence', 'notification_type')} }"
                                  f" emitted {ts:.3f}")
                for rs in b.get("resourceSpans", []):
                    for ss in rs.get("scopeSpans", []):
                        for sp in ss.get("spans", []):
                            print(f"  {round(e['t'], 3):>14} [{e['collector']}] span {sp.get('name')} "
                                  f"start {int(sp['startTimeUnixNano']) / 1e9:.3f} end {int(sp['endTimeUnixNano']) / 1e9:.3f}")
                names = [m.get("name") for rm in b.get("resourceMetrics", []) for sm in rm.get("scopeMetrics", [])
                         for m in sm.get("metrics", [])]
                if names:
                    print(f"  {round(e['t'], 3):>14} [{e['collector']}] metrics {names}")
        print("  files:", [f for f in res["files"] if not f.startswith(("plugins", "statsig"))][:40])
        print("  cc-socks socket present:", res["cc_socks_had_pid"])
        if base and name != "kampr":
            print("  visual diff vs kampr:", json.dumps(diff_snaps(base, res), ensure_ascii=False)[:1500])


def matrix(names):
    out = os.environ.get("KCS_OUT") or tempfile.mkdtemp(prefix="kcs-out-")
    os.makedirs(out, exist_ok=True)
    names = names or list(VARIANTS)
    width = int(os.environ.get("KCS_PARALLEL", "6"))
    pending = list(names)
    running = []
    while pending or running:
        while pending and len(running) < width:
            n = pending.pop(0)
            running.append(subprocess.Popen([sys.executable, os.path.abspath(__file__), "run", n, out]))
        for p in running[:]:
            if p.poll() is not None:
                running.remove(p)
        time.sleep(0.5)
    print("results in", out)
    report(out)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "matrix"
    if cmd == "run":
        run_one(sys.argv[2], sys.argv[3])
    elif cmd == "report":
        report(sys.argv[2])
    else:
        matrix(sys.argv[2:])


if __name__ == "__main__":
    main()
