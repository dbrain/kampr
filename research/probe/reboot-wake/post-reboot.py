#!/usr/bin/env python3
"""Does `wake()` still find the session after the socket its runtime dir held is gone?

Reproduces the post-reboot state without rebooting: a throwaway named session under an isolated
config root, stopped, with everything herdr created for it under the runtime directory removed.
Then asks the two questions manage.rs asks — does `herdr session list --json` still list it, and
does the `socket_path` it reports still match the node's configured socket — and finishes by
respawning the server the way `spawn_server` does and driving one RPC over the recreated socket.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

NAME = "kampr-probe-w0-reboot"

root = tempfile.mkdtemp(prefix="herdr-probe-reboot-")
os.makedirs(root + "/herdr", exist_ok=True)
open(root + "/herdr/config.toml", "w").write("onboarding = false\n")
runtime = tempfile.mkdtemp(prefix="herdr-probe-run-")

env = dict(os.environ,
           XDG_CONFIG_HOME=root,
           XDG_RUNTIME_DIR=runtime,
           HERDR_CONFIG_PATH=root + "/herdr/config.toml")
env.pop("HERDR_SOCKET_PATH", None)
env.pop("HERDR_SESSION", None)

reading = {"herdr": subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip(),
           "configRoot": root, "runtimeDir": runtime}


def herdr(*args, **kw):
    return subprocess.run(["herdr", *args], env=env, capture_output=True, text=True, **kw)


def listing():
    out = herdr("session", "list", "--json")
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"raw": out.stdout, "stderr": out.stderr}


def entry(name=NAME):
    for s in listing().get("sessions", []):
        if s.get("name") == name:
            return s
    return None


def tree(base):
    found = []
    for dirpath, dirnames, filenames in os.walk(base):
        for f in filenames + dirnames:
            p = os.path.join(dirpath, f)
            try:
                st = os.lstat(p)
            except OSError:
                continue
            kind = "sock" if os.path.stat.S_ISSOCK(st.st_mode) else ("dir" if os.path.isdir(p) else "file")
            found.append(f"{kind} {p}")
    return sorted(found)


def answers(path, timeout=2):
    try:
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        c.settimeout(timeout)
        c.connect(path)
        c.sendall(json.dumps({"id": "p", "method": "session.snapshot", "params": {}}).encode() + b"\n")
        line = c.makefile().readline()
        c.close()
        return "result" in json.loads(line) if line.strip() else False
    except (OSError, json.JSONDecodeError):
        return False


def session_at(sessions, sock):
    """The manage.rs helper, transcribed: exact path, else same basename with canonical parents."""
    for s in sessions:
        listed = s.get("socket_path")
        if not listed:
            continue
        if listed == sock:
            return s["name"]
        if os.path.basename(listed) == os.path.basename(sock):
            try:
                if os.path.realpath(os.path.dirname(listed)) == os.path.realpath(os.path.dirname(sock)):
                    return s["name"]
            except OSError:
                pass
    return None


def spawn():
    subprocess.Popen(["herdr", "server", "--session", NAME], env=env,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)


def wait_answering(path, secs=20):
    t0 = time.monotonic()
    while time.monotonic() - t0 < secs:
        if answers(path):
            return round((time.monotonic() - t0) * 1000)
        time.sleep(0.05)
    return None


try:
    spawn()
    listed_sock = None
    t0 = time.monotonic()
    while time.monotonic() - t0 < 20:
        e = entry()
        if e and e.get("running"):
            listed_sock = e.get("socket_path")
            break
        time.sleep(0.05)
    reading["running"] = entry()
    listed_sock = (entry() or {}).get("socket_path")
    reading["firstStartMs"] = wait_answering(listed_sock) if listed_sock else None

    reading["whileRunning"] = {
        "entry": entry(),
        "socketExists": os.path.exists(listed_sock) if listed_sock else None,
        "isSocket": os.path.stat.S_ISSOCK(os.lstat(listed_sock).st_mode) if listed_sock and os.path.exists(listed_sock) else None,
        "underConfigRoot": tree(root),
        "underRuntimeDir": tree(runtime),
    }

    herdr("session", "stop", NAME)
    time.sleep(1.5)
    reading["afterStop"] = {
        "entry": entry(),
        "socketStillOnDisk": os.path.exists(listed_sock),
        "underConfigRoot": tree(root),
        "underRuntimeDir": tree(runtime),
    }

    # The post-reboot state: everything herdr put under the runtime directory is gone, and so is
    # the socket wherever it lived.
    shutil.rmtree(runtime, ignore_errors=True)
    if listed_sock and os.path.exists(listed_sock):
        os.unlink(listed_sock)
    sock_dir = os.path.dirname(listed_sock) if listed_sock else None
    reading["postReboot"] = {
        "removed": [runtime] + ([listed_sock] if listed_sock else []),
        "socketDirStillThere": os.path.isdir(sock_dir) if sock_dir else None,
        "socketOnDisk": os.path.exists(listed_sock) if listed_sock else None,
        "listing": listing(),
        "entry": entry(),
    }
    after = reading["postReboot"]["listing"].get("sessions", [])
    reading["postReboot"]["socketPathSame"] = (entry() or {}).get("socket_path") == listed_sock
    reading["postReboot"]["sessionAtMatches"] = session_at(after, listed_sock)
    reading["postReboot"]["configuredSocket"] = listed_sock

    # A node configured through a symlinked home is the other half of session_at; check the
    # canonicalising branch survives a missing socket file too.
    link_root = None
    if sock_dir:
        link_root = tempfile.mkdtemp(prefix="herdr-probe-link-")
        link = link_root + "/via"
        os.symlink(sock_dir, link)
        reading["postReboot"]["sessionAtViaSymlinkedDir"] = session_at(
            after, os.path.join(link, os.path.basename(listed_sock)))

    spawn()
    reading["respawn"] = {
        "answeredMs": wait_answering(listed_sock, 25),
        "socketRecreatedAtSamePath": os.path.exists(listed_sock),
        "entry": entry(),
        "underRuntimeDir": tree(runtime) if os.path.isdir(runtime) else "runtime dir not recreated",
    }
    if reading["respawn"]["answeredMs"] is not None:
        snap = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "..", "rpc.py"),
                               "workspace.create", json.dumps({"label": "probe", "focus": False})],
                              env=dict(env, HERDR_SOCKET_PATH=listed_sock),
                              capture_output=True, text=True)
        reading["respawn"]["workspaceCreate"] = (snap.stdout or snap.stderr).strip()[:400]
finally:
    herdr("session", "stop", NAME)
    time.sleep(0.5)
    herdr("session", "delete", NAME)
    shutil.rmtree(root, ignore_errors=True)
    shutil.rmtree(runtime, ignore_errors=True)
    if "link_root" in dir() and link_root:
        shutil.rmtree(link_root, ignore_errors=True)

print(json.dumps(reading, indent=2))
