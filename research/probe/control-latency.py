import json, os, socket, subprocess, time, sys
name = f"kprobe-rz-{os.getpid()}"
home = os.path.expanduser("~/.config/herdr")
sock = f"{home}/sessions/{name}/herdr.sock"
env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and not k.startswith("PI_")}
subprocess.Popen(["herdr", "server", "--session", name], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(100):
    if os.path.exists(sock): break
    time.sleep(0.1)
time.sleep(1.0)
def call(method, params):
    s = socket.socket(socket.AF_UNIX); s.connect(sock)
    s.sendall((json.dumps({"id": "p", "method": method, "params": params}) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        buf += s.recv(65536)
    s.close(); return json.loads(buf)
pane = None
call("workspace.create", {"label": "kprobe", "cwd": "/tmp", "focus": False})
for _ in range(100):
    panes = call("session.snapshot", {})["result"]["snapshot"].get("panes", [])
    if panes: pane = panes[0]["pane_id"]; break
    time.sleep(0.1)
time.sleep(1.0)
def rows():
    r = call("pane.get", {"pane_id": pane})
    if "result" not in r: print(r, pane); raise SystemExit
    return r["result"]["pane"]["scroll"]["viewport_rows"]
def until(target, limit=5.0):
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit:
        if rows() == target: return (time.monotonic() - t0) * 1000
        time.sleep(0.002)
    return None
cenv = dict(env, HERDR_SOCKET_PATH=sock)
def claim(c, r):
    t0 = time.monotonic()
    p = subprocess.Popen(["herdr", "terminal", "session", "control", pane, "--cols", str(c), "--rows", str(r)], env=cenv, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    seen = until(r)
    return p, (time.monotonic() - t0) * 1000, seen
def release(p):
    t0 = time.monotonic()
    p.stdin.write(b'{"type":"terminal.release"}\n'); p.stdin.flush(); p.stdin.close(); p.wait()
    return (time.monotonic() - t0) * 1000
try:
    print("pane", pane); print("rows", rows())
    for i in range(5):
        a = 30 + (i % 2) * 6
        p, total, _ = claim(100, a)
        print(f"claim -> {a} rows visible after {total:.0f} ms")
        # in-place resize on the held controller
        b = a + 3
        t0 = time.monotonic()
        p.stdin.write((json.dumps({"type": "terminal.resize", "cols": 110, "rows": b}) + "\n").encode()); p.stdin.flush()
        seen = until(b)
        print(f"  in-place terminal.resize -> {b}: {seen if seen is None else round(seen)} ms")
        # current node path: release, then claim anew
        r_ms = release(p)
        p2, total2, _ = claim(100, a)
        print(f"  release {r_ms:.0f} ms + reclaim {total2:.0f} ms = {r_ms + total2:.0f} ms")
        release(p2)
finally:
    call("server.stop", {})
