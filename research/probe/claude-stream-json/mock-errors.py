#!/usr/bin/env python3
"""A Messages endpoint that only fails: every POST answers <status> with Anthropic's error body.

    mock-errors.py <port> <status> [<error type> [<message>]]
"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port, status = int(sys.argv[1]), int(sys.argv[2])
kind = sys.argv[3] if len(sys.argv) > 3 else {401: "authentication_error", 429: "rate_limit_error",
                                              500: "api_error", 529: "overloaded_error"}.get(status, "invalid_request_error")
message = sys.argv[4] if len(sys.argv) > 4 else kind.replace("_", " ")


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = json.dumps({"type": "error", "error": {"type": kind, "message": message}}).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        sys.stderr.write(f"{self.path} -> {status}\n")

    do_GET = do_POST

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", port), Handler).serve_forever()
