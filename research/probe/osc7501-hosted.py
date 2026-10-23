#!/usr/bin/env python3
"""Whether a native session that boots hosted in a herdr pane gets Claude Code's OSC 7501 channel.

    python3 research/probe/osc7501-hosted.py herdr                 # what herdr answers of `OSC 7501;?` + DA1
    python3 research/probe/osc7501-hosted.py booted <kampr> [...]  # claude booted under `<kampr> attach` in herdr

`herdr` runs a program in a herdr pane that asks `OSC 7501;?` then DA1 and records every byte it
is answered with. `booted` keeps a native session under a throwaway root (`<kampr> new -d`) whose
program waits for herdr to be showing it through `<kampr> attach` and only then execs claude, under
a throwaway CLAUDE_CONFIG_DIR talking to claude-signals.py's mock Messages endpoint. It then types a
turn that asks for a Bash permission at the desk, and reads the session's own log for the 7501
reports. Pass several kampr binaries to compare their attaches (one that answers 7501 against a
build with that answer removed); the first one makes every session. herdr runs under an empty HOME and is stopped after.
"""

import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
spec = importlib.util.spec_from_file_location("signals", os.path.join(HERE, "claude-signals.py"))
signals = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signals)
host = importlib.util.spec_from_file_location("host", os.path.join(HERE, "native-host.py"))
native_host = importlib.util.module_from_spec(host)
host.loader.exec_module(native_host)

REPORT = re.compile(rb"\x1b\]7501;([^\x07\x1b]*)(?:\x07|\x1b\\)")


def herdr():
    work = tempfile.mkdtemp(prefix="k75-", dir=os.environ.get("XDG_RUNTIME_DIR"))
    out = os.path.join(work, "answers")
    prog = os.path.join(work, "ask.py")
    with open(prog, "w") as f:
        f.write(f"""import os, select, termios, time, tty
tty.setraw(0)
os.write(1, b"\\x1b]7501;?\\x07\\x1b[c")
got, end = b"", time.time() + 3
while time.time() < end:
    if select.select([0], [], [], 0.1)[0]:
        got += os.read(0, 256)
open({out!r}, "wb").write(got)
""")
    h = native_host.session("osc7501")
    try:
        p = h.ok("pane.split", direction="right", target_pane_id="w1:p1")["pane"]["pane_id"]
        h.type(p, f" unset HISTFILE; exec {sys.executable} -I {prog}")
        for _ in range(100):
            if os.path.exists(out):
                break
            time.sleep(0.1)
        got = open(out, "rb").read() if os.path.exists(out) else None
        print("herdr", subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip(),
              "answered:", repr(got))
    finally:
        h.stop()
        shutil.rmtree(work, ignore_errors=True)


def log_of(root, sid):
    d = os.path.join(root, sid)
    return b"".join(open(os.path.join(d, n), "rb").read() for n in sorted(os.listdir(d)) if n.startswith("log."))


def booted(kampr, maker):
    mock = signals.serve(signals.Mock, requests=[])
    work = tempfile.mkdtemp(prefix="k75-", dir="/tmp")
    root, cfg, proj = (os.path.join(work, n) for n in ("root", "cfg", "proj"))
    for d in (cfg, proj):
        os.makedirs(d)
    os.makedirs(root, mode=0o700)
    go = os.path.join(work, "go")
    with open(os.path.join(cfg, ".claude.json"), "w") as f:
        json.dump({"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 5,
                   "customApiKeyResponses": {"approved": [signals.KEY[-20:]], "rejected": []},
                   "projects": {proj: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}}, f)
    with open(os.path.join(cfg, "settings.json"), "w") as f:
        json.dump({"tui": "fullscreen", "permissions": {"disableAutoMode": "disable"}}, f)
    env = {k: v for k, v in os.environ.items() if not k.startswith(signals.TERMINAL_ENV)}
    env |= {"CLAUDE_CONFIG_DIR": cfg, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{mock.server_address[1]}",
            "ANTHROPIC_API_KEY": signals.KEY, "DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "KAMPR_KEEP_ROOT": root, "KAMPR_KEEP_LAUNCH": "detached"}
    made = subprocess.run([maker, "new", "-d", "--", "sh", "-c",
                           f"cd {proj}; while [ ! -e {go} ]; do sleep 0.05; done; exec claude"],
                          env=env, capture_output=True, text=True, cwd=proj)
    sid = made.stdout.strip()
    h = native_host.session("osc7501b")
    try:
        p = h.ok("pane.split", direction="right", target_pane_id="w1:p1")["pane"]["pane_id"]
        h.type(p, f" unset HISTFILE; exec env KAMPR_KEEP_ROOT={root} {kampr} attach {sid}")
        hosts = os.path.join(root, sid, "hosts")
        for _ in range(100):
            if os.path.isdir(hosts) and any(n.endswith(".json") for n in os.listdir(hosts)):
                break
            time.sleep(0.1)
        hosted = [json.load(open(os.path.join(hosts, n))) for n in os.listdir(hosts) if n.endswith(".json")]
        print(kampr, subprocess.run([kampr, "--version"], capture_output=True, text=True).stdout.strip(),
              "hosts before boot:", hosted)
        open(go, "w").close()
        t0 = time.time()
        while time.time() - t0 < 25 and "❯" not in h.text(p):
            time.sleep(0.2)
        time.sleep(2)
        h.type(p, "SIGNALS go")
        t1 = time.time()
        while time.time() - t1 < 30 and "Do you want" not in h.text(p):
            time.sleep(0.2)
        time.sleep(1)
        log = log_of(root, sid)
        boot = log.find(b"\x1b]7501;?")
        print("  query at", boot, "| reports:",
              [base64_said(m.group(1)) for m in REPORT.finditer(log) if m.group(1) != b"?"])
        print("  desk shows the dialog:", "Do you want" in h.text(p), "| agent at the desk:",
              {k: h.pane(p).get(k) for k in ("agent", "agent_status")})
    finally:
        h.stop()
        for name in os.listdir(root) if os.path.isdir(root) else []:
            try:
                keeper = json.load(open(os.path.join(root, name, "keeper.json")))
                for pid in (-keeper["child"], keeper["keeper"]):
                    try:
                        os.kill(pid, 9)
                    except OSError:
                        pass
            except (OSError, ValueError):
                pass
        mock.shutdown()
        shutil.rmtree(work, ignore_errors=True)


def base64_said(fields):
    import base64
    out = []
    for field in fields.decode().split(":"):
        k, _, v = field.partition("=")
        out.append(f"{k}={base64.b64decode(v).decode()!r}" if k == "msg" else field)
    return ":".join(o for o in out if not o.startswith("app="))


if __name__ == "__main__":
    if sys.argv[1:2] == ["herdr"]:
        herdr()
    elif sys.argv[1:2] == ["booted"]:
        for kampr in sys.argv[2:]:
            booted(kampr, sys.argv[2])
    else:
        raise SystemExit(__doc__)
