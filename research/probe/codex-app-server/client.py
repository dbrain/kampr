#!/usr/bin/env python3
import json, os, subprocess, sys, threading, time

CODEX = "/home/dbrain/.local/bin/codex"
cdir = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "run")
os.makedirs(cdir, exist_ok=True)
os.makedirs(os.path.join(cdir, "codex-home"), exist_ok=True)

extra_config = [
    "-c", 'model_provider="mock"',
    "-c", 'model="mock-model"',
    "-c", 'model_providers.mock.name="mock"',
    "-c", 'model_providers.mock.base_url="http://127.0.0.1:8765/v1"',
    "-c", 'model_providers.mock.wire_api="responses"',
    "-c", 'model_providers.mock.env_key="MOCK_KEY"',
]

proc = subprocess.Popen(
    [CODEX, "app-server", "--stdio", *extra_config],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    env={**os.environ, "MOCK_KEY": "mock-key", "CODEX_HOME": os.path.join(cdir, "codex-home")},
    text=True, bufsize=1,
)

errlines = []
def drain_err():
    for line in proc.stderr:
        errlines.append(line.rstrip())
        with open(os.path.join(cdir, "stderr.log"), "a") as f:
            f.write(line)
threading.Thread(target=drain_err, daemon=True).start()

out = open(os.path.join(cdir, "wire.log"), "a")
next_id = 100

def send(obj, tag=""):
    line = json.dumps(obj)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()
    out.write(f">> {tag} {line}\n")
    out.flush()

def rpc(method, params=None, tag=""):
    global next_id
    next_id += 1
    msg = {"jsonrpc": "2.0", "id": next_id, "method": method}
    if params is not None:
        msg["params"] = params
    send(msg, tag)
    return next_id

def read_until(deadline=15.0, want_ids=None, stop_methods=None):
    import select
    got = []
    end = time.time() + deadline
    buf = ""
    while time.time() < end:
        remaining = end - time.time()
        r, _, _ = select.select([proc.stdout], [], [], min(remaining, 0.5))
        if r:
            ch = proc.stdout.read(1)
            if not ch:
                break
            buf += ch
            if ch == "\n":
                line = buf.strip()
                buf = ""
                if not line:
                    continue
                out.write(f"<< {line}\n")
                out.flush()
                try:
                    m = json.loads(line)
                except Exception:
                    continue
                got.append(m)
                if want_ids and m.get("id") in want_ids and "result" in m:
                    return got
                if stop_methods and m.get("method") in stop_methods:
                    return got
    return got

def call(method, params=None, tag="", wait=8.0, stop_methods=None):
    rid = rpc(method, params, tag)
    got = read_until(wait, want_ids={rid}, stop_methods=stop_methods)
    for m in got:
        if m.get("id") == rid:
            return m
    return None

# handshake
init = call("initialize", {
    "clientInfo": {"name": "kampr-probe", "version": "0.0.1"},
    "capabilities": {},
}, tag="initialize", wait=10)
print("INIT:", json.dumps(init)[:600])
send({"jsonrpc": "2.0", "method": "initialized"}, tag="initialized")
time.sleep(0.5)

print("DONE-INIT")