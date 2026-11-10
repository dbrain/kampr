# Usage: hook-latency.py <kampr> <shell> <runs>. Times a hook command in the installed form, from
# spawning its shell to the event reaching a socket, as Claude would run it.
import os, socket, subprocess, sys, time, statistics
kampr, shell, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
sock = os.path.join(os.environ.get("TMPDIR", "/tmp"), "hook-latency.sock")
try: os.unlink(sock)
except FileNotFoundError: pass
srv = socket.socket(socket.AF_UNIX); srv.bind(sock); srv.listen(8)
cmd = f"[ -x {kampr} ] && {kampr} hook --socket {sock} 2>/dev/null; exit 0"
event = b'{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"echo"}}'
lat = []
for _ in range(n):
    t = time.monotonic()
    p = subprocess.Popen([shell, "-c", cmd], stdin=subprocess.PIPE)
    p.stdin.write(event); p.stdin.close()
    c, _ = srv.accept()
    data = b""
    while True:
        b = c.recv(65536)
        if not b: break
        data += b
    lat.append((time.monotonic() - t) * 1000)
    c.close(); p.wait()
lat.sort()
print(f"{shell} {os.path.basename(kampr)} n={n} min={lat[0]:.0f} median={statistics.median(lat):.0f} p90={lat[int(n*0.9)-1]:.0f} max={lat[-1]:.0f} ms")
