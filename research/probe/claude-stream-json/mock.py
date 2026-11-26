#!/usr/bin/env python3
"""A Messages endpoint that answers like Anthropic for a headless `claude -p` stream-json test.

    mock.py <port> <control_dir>

Every request is appended to <control_dir>/requests.log so a test can prove the binary reached
the mock and nothing went out to the real API. The next answer is chosen by <control_dir>/mode,
rewritten between turns:

    ok                          a text completion, streamed as Anthropic SSE
    ok <text>                   a text completion of <text>
    overage                     a completion, with the response carrying usage-credit overage
    error <status> [type] [msg] an Anthropic error body, as mock-errors.py returns it

A GET is answered the same as a POST. Any path under /v1/ that is not /v1/messages is a 200 {}.
"""
import json, os, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port, cdir = int(sys.argv[1]), sys.argv[2]
os.makedirs(cdir, exist_ok=True)

def mode():
    try:
        with open(os.path.join(cdir, "mode")) as f:
            return f.read().strip()
    except OSError:
        return "ok"

def log(line):
    with open(os.path.join(cdir, "requests.log"), "a") as f:
        f.write(line + "\n")

def sse(text):
    """The Anthropic Messages SSE completion for one text answer."""
    def ev(name, data):
        return f"event: {name}\ndata: {json.dumps(data)}\n\n"
    return (
        ev("message_start", {"type": "message_start", "message": {
            "id": "msg_mock", "type": "message", "role": "assistant", "model": "mock",
            "content": [], "stop_reason": None, "stop_sequence": None,
            "usage": {"input_tokens": 8, "output_tokens": 0}}})
        + ev("content_block_start", {"type": "content_block_start", "index": 0,
                                     "content_block": {"type": "text", "text": ""}})
        + ev("ping", {"type": "ping"})
        + ev("content_block_delta", {"type": "content_block_delta", "index": 0,
                                     "delta": {"type": "text_delta", "text": text}})
        + ev("content_block_stop", {"type": "content_block_stop", "index": 0})
        + ev("message_delta", {"type": "message_delta",
                               "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                               "usage": {"output_tokens": 15}})
        + ev("message_stop", {"type": "message_stop"}))

def tool_sse():
    """The Anthropic Messages SSE completion for a single tool_use block: `claude -p` will
    answer it with a `can_use_tool` control request when it is in a mode that asks."""
    def ev(name, data):
        return f"event: {name}\ndata: {json.dumps(data)}\n\n"
    return (
        ev("message_start", {"type": "message_start", "message": {
            "id": "msg_mock", "type": "message", "role": "assistant", "model": "mock",
            "content": [], "stop_reason": None, "stop_sequence": None,
            "usage": {"input_tokens": 8, "output_tokens": 0}}})
        + ev("content_block_start", {"type": "content_block_start", "index": 0,
                                     "content_block": {"type": "tool_use",
                                                       "id": "toolu_mock", "name": "Bash",
                                                       "input": {}}})
        + ev("input_json_delta", {"type": "input_json_delta", "index": 0,
                                  "delta": {"type": "input_json_delta",
                                            "partial_json": "{\"command\": \"echo mock\"}"}})
        + ev("content_block_stop", {"type": "content_block_stop", "index": 0})
        + ev("message_delta", {"type": "message_delta",
                               "delta": {"stop_reason": "tool_use", "stop_sequence": None},
                               "usage": {"output_tokens": 15}})
        + ev("message_stop", {"type": "message_stop"}))

class Handler(BaseHTTPRequestHandler):
    def _body(self):
        n = int(self.headers.get("content-length", 0))
        return self.rfile.read(n) if n else b""

    def _answer(self):
        self._body()
        log(f"{self.command} {self.path} {self.headers.get('x-api-key', '')[:12]}")
        m = mode().split()
        if m and m[0] == "error":
            status = int(m[1]) if len(m) > 1 else 500
            kind = m[2] if len(m) > 2 else {"401": "authentication_error", "429": "rate_limit_error",
                                             "500": "api_error", "529": "overloaded_error"}.get(str(status), "api_error")
            msg = " ".join(m[3:]) if len(m) > 3 else kind.replace("_", " ")
            body = json.dumps({"type": "error", "error": {"type": kind, "message": msg}}).encode()
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        text = " ".join(m[1:]) if m and len(m) > 1 else "Hello from the mock."
        if m and m[0] == "tool":
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            self.wfile.write(tool_sse().encode())
            self.wfile.flush()
            return
        if m and m[0] == "overage":
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("anthropic-ratelimit-requests-remaining", "0")
            self.send_header("anthropic-ratelimit-tokens-remaining", "0")
            self.end_headers()
            self.wfile.write(sse(text).encode())
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        self.wfile.write(sse(text).encode())
        self.wfile.flush()

    def do_POST(self):
        self._answer()

    def do_GET(self):
        self._answer()

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()