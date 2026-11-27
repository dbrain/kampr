#!/usr/bin/env python3
import json, os, sys, time
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

def sse(events):
    body = ""
    for name, data in events:
        body += f"event: {name}\ndata: {json.dumps(data)}\n\n"
    return body.encode()

def resp_obj(text, status="completed"):
    return {
        "id": "resp_mock",
        "object": "response",
        "created_at": 1700000000,
        "status": status,
        "model": "mock-model",
        "output": [
            {
                "type": "message",
                "id": "msg_mock",
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": text, "annotations": []}],
            }
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
    }

def stream_events(text):
    r = resp_obj(text)
    msg = r["output"][0]
    part = msg["content"][0]
    return [
        ("response.created", {"type": "response.created", "response": {**r, "status": "in_progress", "output": []}}),
        ("response.in_progress", {"type": "response.in_progress", "response": {**r, "status": "in_progress", "output": []}}),
        ("response.output_item.added", {"type": "response.output_item.added", "output_index": 0, "item": {**msg, "content": []}}),
        ("response.content_part.added", {"type": "response.content_part.added", "item_id": msg["id"], "output_index": 0, "content_index": 0, "part": {**part, "text": ""}}),
        ("response.output_text.delta", {"type": "response.output_text.delta", "item_id": msg["id"], "output_index": 0, "content_index": 0, "delta": text}),
        ("response.output_text.done", {"type": "response.output_text.done", "item_id": msg["id"], "output_index": 0, "content_index": 0, "text": text}),
        ("response.content_part.done", {"type": "response.content_part.done", "item_id": msg["id"], "output_index": 0, "content_index": 0, "part": part}),
        ("response.output_item.done", {"type": "response.output_item.done", "output_index": 0, "item": msg}),
        ("response.completed", {"type": "response.completed", "response": r}),
    ]

def slow_events(text):
    r = resp_obj(text)
    msg = r["output"][0]
    part = msg["content"][0]
    head = [
        ("response.created", {"type": "response.created", "response": {**r, "status": "in_progress", "output": []}}),
        ("response.in_progress", {"type": "response.in_progress", "response": {**r, "status": "in_progress", "output": []}}),
        ("response.output_item.added", {"type": "response.output_item.added", "output_index": 0, "item": {**msg, "content": []}}),
        ("response.content_part.added", {"type": "response.content_part.added", "item_id": msg["id"], "output_index": 0, "content_index": 0, "part": {**part, "text": ""}}),
    ]
    words = text.split()
    mids = []
    for i, w in enumerate(words):
        mids.append(("response.output_text.delta", {"type": "response.output_text.delta", "item_id": msg["id"], "output_index": 0, "content_index": 0, "delta": w + " "}))
    tail = [
        ("response.output_text.done", {"type": "response.output_text.done", "item_id": msg["id"], "output_index": 0, "content_index": 0, "text": text}),
        ("response.content_part.done", {"type": "response.content_part.done", "item_id": msg["id"], "output_index": 0, "content_index": 0, "part": part}),
        ("response.output_item.done", {"type": "response.output_item.done", "output_index": 0, "item": msg}),
        ("response.completed", {"type": "response.completed", "response": r}),
    ]
    return head, mids, tail

class Handler(BaseHTTPRequestHandler):
    def _body(self):
        n = int(self.headers.get("content-length", 0))
        return self.rfile.read(n) if n else b""

    def _answer(self):
        raw = self._body()
        try:
            parsed = json.loads(raw) if raw else {}
        except Exception:
            parsed = {}
        log(f"{self.command} {self.path} model={parsed.get('model')} keys={sorted(parsed.keys())}")
        with open(os.path.join(cdir, "requests-raw.log"), "a") as f:
            f.write(json.dumps(parsed, sort_keys=True) + "\n")
        m = mode().split()
        if m and m[0] == "error":
            status = int(m[1]) if len(m) > 1 else 500
            msg = " ".join(m[2:]) if len(m) > 2 else "mock error"
            body = json.dumps({"error": {"message": msg, "type": "mock_error", "code": str(status)}}).encode()
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        text = " ".join(m[1:]) if m and len(m) > 1 else "Hello from the mock provider."
        if m and m[0] == "slow":
            head, mids, tail = slow_events(text)
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            for name, data in head:
                self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())
                self.wfile.flush()
            for name, data in mids:
                time.sleep(1.0)
                self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())
                self.wfile.flush()
            for name, data in tail:
                self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())
                self.wfile.flush()
            return
        data = sse(stream_events(text))
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        self.wfile.flush()

    def do_POST(self):
        self._answer()

    def do_GET(self):
        self._answer()

    def log_message(self, *args):
        pass

ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()