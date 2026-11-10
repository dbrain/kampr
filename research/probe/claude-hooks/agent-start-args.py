#!/usr/bin/env python3
"""Does herdr's agent.start hand `args` to the harness verbatim: a JSON string with quotes, spaces,
`;` and `$` in it?

A throwaway herdr server under a scratch config root; agent.start {kind: "claude", args} on its
first pane; the `claude` process in that directory has its /proc cmdline compared with the args.
"""
import json, os, re, socket, subprocess, tempfile, time

root = tempfile.mkdtemp(prefix="kampr-probe-args-")
env = dict(os.environ, XDG_CONFIG_HOME=root, HERDR_CONFIG_PATH=root + "/herdr/config.toml")
env.pop("HERDR_SOCKET_PATH", None)
env.pop("HERDR_SESSION", None)
sock = root + "/herdr/herdr.sock"
server = subprocess.Popen(["herdr", "server"], env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


def rpc(method, params):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(10)
    s.connect(sock)
    s.sendall((json.dumps({"id": "p", "method": method, "params": params}) + "\n").encode())
    line = s.makefile().readline()
    s.close()
    return json.loads(line)


try:
    for _ in range(500):
        try:
            rpc("session.snapshot", {})
            break
        except OSError:
            time.sleep(0.02)
    rpc("workspace.create", {"cwd": root, "focus": False})
    pane = re.findall(r'"pane_id": ?"([^"]+)"', json.dumps(rpc("session.snapshot", {})))[0]
    settings = json.dumps({"hooks": {"Stop": [{"hooks": [{"type": "command",
        "command": "[ -x '/a b/kampr' ] && '/a b/kampr' hook --socket '/x y.sock' 2>/dev/null; exit 0"}]}]}})
    want = ["--settings", settings, "--model", "it's $HOME"]
    for _ in range(20):
        r = rpc("agent.start", {"pane_id": pane, "kind": "claude", "name": "claude", "args": want})
        if "error" not in r:
            break
        time.sleep(0.3)
    time.sleep(3)
    got = None
    for pid in filter(str.isdigit, os.listdir("/proc")):
        try:
            if os.readlink(f"/proc/{pid}/cwd") == root and open(f"/proc/{pid}/comm").read().strip() == "claude":
                got = [a.decode() for a in open(f"/proc/{pid}/cmdline", "rb").read().split(b"\0")[1:-1]]
        except OSError:
            pass
    print("verbatim:", got == want)
    print("got:", json.dumps(got))
finally:
    os.killpg(server.pid, 15)
