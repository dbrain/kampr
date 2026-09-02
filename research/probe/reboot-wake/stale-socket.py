#!/usr/bin/env python3
"""The reboot-specific half: a socket file the kernel never cleaned up.

`herdr session stop` unlinks the socket, but a reboot does not — the session directory is under
the config root, which is persistent, so a machine that went down hard comes back up with the
socket file still on disk and nothing listening on it. Measures what `herdr session list --json`
says about a session in that state, and whether `herdr server --session <name>` will start over it.
Also measures a config root herdr has never run in, which is what a brand-new host looks like.
"""
import json, os, shutil, signal, socket, subprocess, sys, tempfile, time

NAME = "kampr-probe-w0-stale"


def root():
    d = tempfile.mkdtemp(prefix="herdr-probe-stale-")
    os.makedirs(d + "/herdr", exist_ok=True)
    open(d + "/herdr/config.toml", "w").write("onboarding = false\n")
    return d, dict(os.environ, XDG_CONFIG_HOME=d, HERDR_CONFIG_PATH=d + "/herdr/config.toml")


def clean(env):
    for k in ("HERDR_SOCKET_PATH", "HERDR_SESSION"):
        env.pop(k, None)
    return env


def listing(env):
    out = subprocess.run(["herdr", "session", "list", "--json"], env=env, capture_output=True, text=True)
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"raw": out.stdout.strip(), "stderr": out.stderr.strip()}


def entry(env, name=NAME):
    for s in listing(env).get("sessions", []):
        if s.get("name") == name:
            return s
    return None


def answers(path, timeout=2):
    try:
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        c.settimeout(timeout)
        c.connect(path)
        c.sendall(b'{"id":"p","method":"session.snapshot","params":{}}\n')
        line = c.makefile().readline()
        c.close()
        return "result" in json.loads(line) if line.strip() else False
    except (OSError, json.JSONDecodeError):
        return False


def server_pids(env):
    out = subprocess.run(["pgrep", "-af", "herdr server"], capture_output=True, text=True).stdout
    return [l for l in out.splitlines() if NAME in l]


reading = {"herdr": subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip()}

fresh, fenv = root()
clean(fenv)
reading["freshConfigRoot"] = {"listing": listing(fenv)}
shutil.rmtree(fresh, ignore_errors=True)

d, env = root()
clean(env)
try:
    subprocess.Popen(["herdr", "server", "--session", NAME], env=env, stdin=subprocess.DEVNULL,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    sock = d + f"/herdr/sessions/{NAME}/herdr.sock"
    t0 = time.monotonic()
    while time.monotonic() - t0 < 20 and not answers(sock):
        time.sleep(0.05)
    reading["running"] = {"entry": entry(env), "answers": answers(sock)}

    pids = [int(l.split()[0]) for l in server_pids(env)]
    reading["killed"] = {"pids": pids}
    for p in pids:
        os.kill(p, signal.SIGKILL)
    time.sleep(1.0)

    reading["afterSigkill"] = {
        "socketStillOnDisk": os.path.exists(sock),
        "isSocket": os.path.stat.S_ISSOCK(os.lstat(sock).st_mode) if os.path.exists(sock) else None,
        "connectRefused": None,
        "answers": answers(sock),
        "entry": entry(env),
        "sessionDir": sorted(os.listdir(os.path.dirname(sock))),
    }
    try:
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        c.settimeout(2)
        c.connect(sock)
        reading["afterSigkill"]["connectRefused"] = False
        c.close()
    except OSError as e:
        reading["afterSigkill"]["connectRefused"] = f"{type(e).__name__}: {e}"

    # `spawn_server`'s exact shape — detached, all three stdio null — over the stale socket.
    started = time.monotonic()
    child = subprocess.Popen(["herdr", "server", "--session", NAME], env=env,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL, start_new_session=True)
    took = None
    while time.monotonic() - started < 25:
        if answers(sock):
            took = round((time.monotonic() - started) * 1000)
            break
        time.sleep(0.05)
    reading["startOverStaleSocket"] = {
        "answeredMs": took,
        "childExit": child.poll(),
        "entryAfter": entry(env),
        "log": open(os.path.dirname(sock) + "/herdr-server.log").read()[-1200:],
    }
    if took is not None:
        snap = subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "rpc.py"),
                               "workspace.create", '{"label":"probe","focus":false}'],
                              env=dict(env, HERDR_SOCKET_PATH=sock), capture_output=True, text=True)
        reading["startOverStaleSocket"]["workspaceCreate"] = (snap.stdout or snap.stderr).strip()[:200]

finally:
    subprocess.run(["herdr", "session", "stop", NAME], env=env, capture_output=True)
    time.sleep(0.5)
    subprocess.run(["herdr", "session", "delete", NAME], env=env, capture_output=True)
    for l in server_pids(env):
        try:
            os.kill(int(l.split()[0]), signal.SIGKILL)
        except (OSError, ValueError):
            pass
    shutil.rmtree(d, ignore_errors=True)

print(json.dumps(reading, indent=2))
