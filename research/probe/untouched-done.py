"""Whether an untouched, unfocused Claude pane ever turns `done` on its own.

    python3 research/probe/untouched-done.py fresh     # start claude, never type, watch
    python3 research/probe/untouched-done.py turn      # one turn, then leave it alone, watch
    python3 research/probe/untouched-done.py resume    # `claude --continue` on a directory with history

Samples herdr's `agent_status` and the session marker's `status` every 100 ms and prints each
transition with its time since the claude process appeared. A throwaway named session, stopped and
removed on the way out; the claude it starts has no CLAUDE* variables, so it writes a transcript.
"""

import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc

SESSIONS = os.path.expanduser("~/.config/herdr/sessions")
AGENT = "w2:p1"


def clean_env():
    return {k: v for k, v in os.environ.items() if not k.startswith("CLAUDE") and k != "HERDR_SOCKET_PATH"}


def start(name):
    sock = os.path.join(SESSIONS, name, "herdr.sock")
    subprocess.Popen(
        ["herdr", "server", "--session", name],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        env=clean_env(),
    )
    for _ in range(200):
        if os.path.exists(sock):
            time.sleep(0.3)
            return sock
        time.sleep(0.1)
    raise SystemExit(f"herdr never opened a socket for {name}")


def stop(name):
    directory = os.path.join(SESSIONS, name)
    sock = os.path.join(directory, "herdr.sock")
    try:
        rpc("server.stop", {}, sock_path=sock)
    except OSError:
        pass
    for _ in range(50):
        if not os.path.exists(sock):
            break
        time.sleep(0.1)
    shutil.rmtree(directory, ignore_errors=True)


def pane(sock):
    return rpc("pane.get", {"pane_id": AGENT}, sock_path=sock)["result"]["pane"]


def claude_pid(sock):
    info = rpc("pane.process_info", {"pane_id": AGENT}, sock_path=sock)["result"]["process_info"]
    for p in info.get("foreground_processes", []):
        if p["name"] == "claude":
            return p["pid"]
    return None


def marker(pid):
    try:
        with open(os.path.expanduser(f"~/.claude/sessions/{pid}.json")) as f:
            m = json.load(f)
        return m.get("status"), m.get("sessionId")
    except (OSError, ValueError):
        return None, None


def transcript_mtime(session):
    if not session:
        return None
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}.jsonl"))
    return round(os.path.getmtime(hits[0]), 1) if hits else None


def screen(sock):
    r = rpc("pane.read", {"pane_id": AGENT, "source": "visible", "lines": 80, "format": "text"}, sock_path=sock)
    return r.get("result", {}).get("read", {}).get("text", "") if r else ""


def watch(sock, seconds, t0, pid, label):
    last = None
    deadline = time.time() + seconds
    while time.time() < deadline:
        p = pane(sock)
        status, session = marker(pid)
        mtime = transcript_mtime(session)
        now = (p.get("agent_status"), p.get("agent"), status, mtime)
        if now != last:
            age = f"{time.time() - mtime:.1f}s ago" if mtime else "-"
            print(f"  [{label}] t={time.time() - t0:7.2f}s herdr={now[0]!s:<8} agent={now[1]!s:<7} marker={now[2]} transcript_mtime={age}", flush=True)
            last = now
        time.sleep(0.1)
    return last


def launch(sock, cmd):
    rpc("pane.send_text", {"pane_id": AGENT, "text": cmd}, sock_path=sock)
    t0 = time.time()
    pid = None
    while pid is None and time.time() - t0 < 30:
        pid = claude_pid(sock)
        time.sleep(0.05)
    print(f"claude pid {pid} after {time.time() - t0:.2f}s ({cmd.strip()})")
    booting = time.time()
    trusted = False
    while time.time() - booting < 20:
        text = screen(sock)
        if not trusted and "trust" in text.lower() and "folder" in text.lower():
            rpc("pane.send_keys", {"pane_id": AGENT, "keys": ["Enter"]}, sock_path=sock)
            trusted = True
            print(f"  answered trust prompt at t={time.time() - t0:.2f}s")
        if "❯" in text and "trust" not in text.lower():
            break
        time.sleep(0.2)
    return t0, pid


def submit(sock):
    rpc("pane.send_text", {"pane_id": AGENT, "text": "reply with the single word ok"}, sock_path=sock)
    time.sleep(0.3)
    rpc("pane.send_keys", {"pane_id": AGENT, "keys": ["Enter"]}, sock_path=sock)


def main(mode, seconds):
    name = f"kampr-probe-untouched-{os.getpid()}"
    work = tempfile.mkdtemp(prefix="kampr-untouched-")
    sock = start(name)
    try:
        rpc("workspace.create", {"label": "desk", "cwd": "/tmp"}, sock_path=sock)
        rpc("workspace.create", {"label": "agent", "cwd": work, "focus": False}, sock_path=sock)
        time.sleep(0.5)
        focused = [p["pane_id"] for p in rpc("pane.list", {}, sock_path=sock)["result"]["panes"] if p.get("focused")]
        print(f"focused panes: {focused}")
        t0, pid = launch(sock, "claude\n")
        if mode in ("turn", "resume"):
            watch(sock, 8, t0, pid, "boot")
            submit(sock)
            print(f"  prompt submitted at t={time.time() - t0:.2f}s")
        if mode == "resume":
            watch(sock, 30, t0, pid, "turn")
            rpc("pane.send_text", {"pane_id": AGENT, "text": "/exit"}, sock_path=sock)
            time.sleep(0.3)
            rpc("pane.send_keys", {"pane_id": AGENT, "keys": ["Enter"]}, sock_path=sock)
            gone = time.time()
            while claude_pid(sock) and time.time() - gone < 15:
                time.sleep(0.1)
            print(f"  claude exited; herdr now {pane(sock).get('agent_status')}")
            time.sleep(3)
            t0, pid = launch(sock, "claude --continue\n")
        final = watch(sock, seconds, t0, pid, mode)
        print("final screen tail:")
        print("\n".join(screen(sock).rstrip().splitlines()[-8:]))
        print(f"final: {final}")
    finally:
        stop(name)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "fresh", float(sys.argv[2]) if len(sys.argv) > 2 else 90)
