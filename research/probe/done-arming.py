"""When a `pane.report_agent` arms herdr's `done`, and when it does not (#405).

    python3 research/probe/done-arming.py window   # the gap sweep, anchored inside the hold
    python3 research/probe/done-arming.py gated    # the same sweep, anchored past herdr's own publish
    python3 research/probe/done-arming.py control  # a pane herdr never labelled, and a re-report

Every run is its own throwaway named session, stopped and removed on the way out.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rpc import rpc

SESSIONS = os.path.expanduser("~/.config/herdr/sessions")
PANE = "w2:p1"


def start(name):
    sock = os.path.join(SESSIONS, name, "herdr.sock")
    subprocess.Popen(
        ["herdr", "server", "--session", name],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
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
    return rpc("pane.get", {"pane_id": PANE}, sock_path=sock)["result"]["pane"]


def report(sock, state):
    rpc(
        "pane.report_agent",
        {"pane_id": PANE, "agent": "claude", "source": "kampr-probe", "state": state},
        sock_path=sock,
    )


def until_done(sock, seconds=5.0):
    deadline = time.time() + seconds
    saw = None
    while time.time() < deadline:
        saw = pane(sock)["agent_status"]
        if saw == "done":
            return saw
        time.sleep(0.05)
    return saw


def run(tag, gap, anchor="hold", binary="claude", requarm=False, hold_after=0.0):
    """`anchor`: 'hold' reports as soon as the process is in the foreground — inside herdr's
    post-label `unknown` window; 'published' waits for herdr's own first answer about the pane."""
    name = "kampr-probe-donearm-{}-{}".format(os.getpid(), "".join(c if c.isalnum() else "-" for c in tag))
    sock = start(name)
    work = tempfile.mkdtemp()
    try:
        shutil.copy(shutil.which("sleep"), os.path.join(work, binary))
        rpc("workspace.create", {"label": "kampr", "cwd": "/tmp"}, sock_path=sock)
        rpc("workspace.create", {"label": "convo", "cwd": work}, sock_path=sock)
        time.sleep(0.4)
        at = time.time()
        if binary:
            rpc("pane.send_text", {"pane_id": PANE, "text": f"{work}/{binary} 600\n"}, sock_path=sock)
            while True:
                info = rpc("pane.process_info", {"pane_id": PANE}, sock_path=sock)["result"]["process_info"]
                if any(p["name"] == binary for p in info.get("foreground_processes", [])):
                    break
                time.sleep(0.05)
        if anchor == "published":
            while time.time() - at < 30:
                read = pane(sock)
                if read.get("agent") == "claude" and read.get("agent_status") == "idle":
                    break
                time.sleep(0.05)
        published = round(time.time() - at, 3)
        report(sock, "working")
        while pane(sock)["agent_status"] != "working":
            time.sleep(0.02)
        time.sleep(gap)
        if requarm:
            report(sock, "working")
        report(sock, "idle")
        settled = until_done(sock)
        held = set()
        deadline = time.time() + hold_after
        while time.time() < deadline:
            held.add(pane(sock)["agent_status"])
            time.sleep(0.5)
        print(
            f"  {tag:<28} gap={gap:<5} anchor={anchor:<9} at={published:<6} -> {settled}"
            + (f"  held={sorted(held)}" if held else "")
        )
        return settled
    finally:
        stop(name)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "window"
    if what == "window":
        for gap in (0, 0.05, 0.1, 0.2, 0.3, 0.4, 0.45, 0.5, 0.6, 0.8, 1.2, 2, 4, 8):
            run(f"hold-{gap}", gap)
    elif what == "gated":
        for gap in (0, 0.5, 2, 5, 10):
            for run_number in range(2):
                run(f"published-{gap}-{run_number}", gap, anchor="published", hold_after=20)
    elif what == "control":
        for gap in (2, 20):
            run(f"unlabelled-{gap}", gap, binary="nap")
        for gap in (2,):
            run(f"re-report-{gap}", gap, requarm=True)
    else:
        raise SystemExit(__doc__)
