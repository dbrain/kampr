#!/usr/bin/env python3
"""What muffit's TUI makes of an image path pasted bracketed into the middle of its box, how its
box edits around the chip it draws, and whether a line sent the way Kampr's composer sends one
reaches the model with the picture in it.

An isolated muffit: its own MUFFIT_HOME, config, port and database, and a fake OpenAI endpoint
that answers `ok` and records every request body, so the operator's real daemon is never touched.
The TUI runs in a throwaway named herdr session.

Usage: tui-image.py <image>
"""
import http.server, json, os, shutil, socket, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "mirror"))
from common import Session

IMAGE = sys.argv[1]
BODIES = []


def sse(text, stop):
    chunk = {"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "test-model",
             "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": None}]}
    out = f"data: {json.dumps(chunk)}\n\n"
    if stop:
        done = {"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "test-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}
        out += f"data: {json.dumps(done)}\n\ndata: [DONE]\n\n"
    return out


class Fake(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        body = b'{"data":[{"id":"test-model"}]}'
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        BODIES.append(json.loads(self.rfile.read(n) or b"{}"))
        body = sse("ok", True).encode()
        self.send_response(200); self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)


def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def last_user(body):
    for m in reversed(body.get("messages", [])):
        if m.get("role") == "user":
            c = m.get("content")
            if isinstance(c, str):
                return [("text", c[:120])]
            return [(b.get("type"), (b.get("text") or "")[:120]) for b in c]
    return None


def main():
    fake = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fake)
    threading.Thread(target=fake.serve_forever, daemon=True).start()
    root = tempfile.mkdtemp(prefix="kampr-muffit-")
    port = free_port()
    config = os.path.join(root, "muffit.toml")
    open(config, "w").write(f'''[provider]
kind = "openai"
model = "test-model"
base_url = "http://127.0.0.1:{fake.server_address[1]}"
api_key = "test-key"

[server]
host = "127.0.0.1"
port = {port}
token = "test-token"

[staging]
tier_b = "off"

[storage]
path = "{root}/muffit.db"
''')
    env = f"MUFFIT_CONFIG_FILE={config} MUFFIT_HOME={root}/home"
    try:
        with Session("muffit") as s:
            p = s.pane()
            s.send(p, f"cd {s.cwd} && {env} muffit\n")
            time.sleep(8)
            print("== booted\n" + s.screen(p)[-1200:], flush=True)

            def show(label):
                print(f"== {label}\n" + "\n".join(r for r in s.screen(p).splitlines()[-8:] if r.strip()), flush=True)

            for ch in "alpha beta gamma":
                s.send(p, ch); time.sleep(0.03)
            time.sleep(0.5)
            s.send(p, "\x1b[D" * 6); time.sleep(0.5)
            s.send(p, f"\x1b[200~{IMAGE}\x1b[201~"); time.sleep(1.5)
            show("pasted mid-line")
            s.send(p, "\x7f"); time.sleep(0.8)
            show("one backspace after it")
            s.send(p, "\x7f"); time.sleep(0.8)
            show("a second backspace")
            s.send(p, "\x15"); time.sleep(0.5)
            s.send(p, "\x7f" * 40); time.sleep(0.8)

            for label, writes in [
                ("chip after the words, Enter as its own write", ["first look", f" \x1b[200~{IMAGE}\x1b[201~", "\r"]),
                ("chip mid-line by arrows, then Enter", ["second look here", "\x1b[D" * 5, f"\x1b[200~{IMAGE}\x1b[201~", "\r"]),
            ]:
                before = len(BODIES)
                for w in writes:
                    s.send(p, w)
                time.sleep(6)
                show(f"sent: {label}")
                print("   model saw:", [last_user(b) for b in BODIES[before:]], flush=True)
            s.send(p, "\x03"); time.sleep(0.5); s.send(p, "\x03"); time.sleep(1)
    finally:
        subprocess.run(["pkill", "-f", f"MUFFIT_HOME={root}/home"], check=False)
        subprocess.run(f"MUFFIT_CONFIG_FILE={config} MUFFIT_HOME={root}/home muffit server stop", shell=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(root, ignore_errors=True)
        fake.shutdown()


if __name__ == "__main__":
    main()
