"""What herdr does with a restored pane whose stored resume_argv names a session the machine lost (docs/25, "The stale attach"; probes #694, #695).

    python3 research/probe/stale-attach.py <kampr binary> plain      # the argv a node stored before the fix: attach <dead id>
    python3 research/probe/stale-attach.py <kampr binary> restored   # attach --restored <dead id>
    python3 research/probe/stale-attach.py <kampr binary> replace    # what a report without resume_argv does to the stored entry

A reboot is stood in for by an empty session root: the keeper directory the argv names does not exist.
Runs herdr under an empty HOME and tears it down; never touches a real session.
"""

import glob
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("native_host", os.path.join(HERE, "native-host.py"))
nh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nh)

DEAD = "01dead0000000000000000000a"


def stored():
    out = []
    pattern = os.path.join(nh.EMPTY_HOME, ".config/herdr/**/session*.json")
    for path in glob.glob(pattern, recursive=True):
        for workspace in json.load(open(path)).get("workspaces", []):
            for tab in workspace.get("tabs", []):
                for key, pane in tab.get("panes", {}).items():
                    out.append((os.path.relpath(path, nh.EMPTY_HOME), key, pane.get("agent_resume")))
    return out


def observe(h, label):
    for x in h.ok("pane.list")["panes"]:
        pid = x["pane_id"]
        print(f"  {label} {pid}: agent={x.get('agent')} status={x['agent_status']} restore_error={x.get('restore_error')} fg={h.fg(pid)}")
        print("    keys", sorted(x))
        print("    visible:", repr(h.text(pid, "visible", "text")[-400:]))


ROOT = os.path.join(nh.WORK, "root")


def keeper(binary):
    env = dict(os.environ, KAMPR_KEEP_ROOT=ROOT, KAMPR_KEEP_LAUNCH="detached", HOME=nh.EMPTY_HOME, SHELL="/usr/bin/bash")
    os.makedirs(ROOT, mode=0o700, exist_ok=True)
    out = subprocess.run([binary, "new", "-d", "--cwd", "/tmp", "--", "bash", "--norc"], env=env, capture_output=True, text=True, check=True)
    return out.stdout.strip()


def reboot():
    """What a boot does to the keepers: the processes and the tmpfs directory they lived in are gone."""
    for pid in os.listdir("/proc"):
        try:
            cmd = open(f"/proc/{pid}/cmdline").read().split("\0")
        except OSError:
            continue
        if pid.isdigit() and ROOT in cmd[-1:] + cmd[1:3]:
            os.kill(int(pid), signal.SIGKILL)
    time.sleep(0.5)
    shutil.rmtree(ROOT)
    os.makedirs(ROOT, mode=0o700)


def session(binary, marker):
    sid = keeper(binary)
    h = nh.Herdr(f"kph-stale-{os.getpid()}")
    h.env["PATH"] = os.environ["PATH"]
    argv = ["env", f"KAMPR_KEEP_ROOT={ROOT}", binary, "attach", *marker, sid]
    return h, argv


def restore(binary, variant):
    h, argv = session(binary, ["--restored"] if variant == "restored" else [])
    try:
        h.start()
        h.ok("workspace.create")
        time.sleep(1)
        print("attach in the pane:", h.run("w1:p1", " ".join(argv)))
        print("report_agent", h("pane.report_agent", pane_id="w1:p1", source="kampr", agent="kampr", state="idle", resume_argv=argv))
        for n, how in enumerate(["stop", "stop", "sigkill"]):
            nh.kill(h, how)
            if n == 0:
                reboot()
            print(f"{variant} after {how}: stored", [(k, a and a["argv"][3:]) for p, k, a in stored() if "snapshots" not in p])
            h.start()
            time.sleep(4)
            observe(h, how)
            print("  stored after restore", [(k, a and a["argv"][3:]) for p, k, a in stored() if "snapshots" not in p])
            h.type("w1:p1", "pwd; echo STILL-A-SHELL")
            time.sleep(1)
            print("  typed into the pane:", repr(h.text("w1:p1", "visible", "text")[-200:]))
    finally:
        h.stop()


def replace(binary):
    h, argv = session(binary, [])
    seq = [int(time.time() * 1e6)]

    def report(**extra):
        seq[0] += 1
        return h("pane.report_agent", pane_id="w1:p1", source="kampr", agent="kampr", state="idle", seq=seq[0], **extra)

    variants = {
        "no resume_argv": lambda: report(),
        "release_agent": lambda: h("pane.release_agent", pane_id="w1:p1", source="kampr", agent="kampr"),
        "resume_argv ['true']": lambda: report(resume_argv=["true"]),
        "resume_argv []": lambda: report(resume_argv=[]),
    }
    try:
        h.start()
        h.ok("workspace.create")
        time.sleep(1)
        h.run("w1:p1", " ".join(argv))
        report(resume_argv=argv)
        print("pane.get keys after a report with resume_argv:", sorted(h.pane("w1:p1")))
        for name, apply in variants.items():
            report(resume_argv=argv)
            print(f"{name} ->", apply())
            h("server.stop")
            h.proc.wait(5)
            print("  stored:", [(k, a and a["argv"][3:]) for p, k, a in stored() if "snapshots" not in p])
            h.start()
            time.sleep(3)
    finally:
        h.stop()


if __name__ == "__main__":
    binary, variant = os.path.abspath(sys.argv[1]), sys.argv[2]
    try:
        replace(binary) if variant == "replace" else restore(binary, variant)
    finally:
        shutil.rmtree(nh.WORK, ignore_errors=True)
