#!/usr/bin/env python3
import sys, threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer


class Handler(SimpleHTTPRequestHandler):
    sink = None

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n).decode("utf-8", "replace")
        print(body, flush=True)
        if Handler.sink:
            with open(Handler.sink, "a") as f:
                f.write(body + "\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, *a):
        pass


def main():
    root = sys.argv[1]
    port = int(sys.argv[2])
    Handler.sink = sys.argv[3] if len(sys.argv) > 3 else None
    srv = ThreadingHTTPServer(("127.0.0.1", port), partial(Handler, directory=root))
    print(f"serving {root} on {port}", flush=True)
    srv.serve_forever()


main()
