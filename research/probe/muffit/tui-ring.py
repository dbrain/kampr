#!/usr/bin/env python3
"""What herdr reports for a pane running muffit's TUI: its agent label, the foreground process the
node names the pane by, and the scrollback ring while the TUI holds the alternate screen (#244).

An isolated muffit (own HOME, MUFFIT_HOME, config, port, token and database; a provider URL nothing
listens on, and no input is typed) in a throwaway herdr session. Two cases: `muffit`, which starts
its own server, and `muffit-tui --server` against a `muffit-server` started first.

Usage: tui-ring.py
"""
import json, os, shutil, socket, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "mirror"))
from common import Session


def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def reading(s, p):
    pane = s.call("pane.get", {"pane_id": p})["pane"]
    info = s.call("pane.process_info", {"pane_id": p})["process_info"]
    fg = [(f.get("name"), f.get("cmdline") or " ".join(f.get("argv") or [])) for f in info.get("foreground_processes", [])]
    return {"agent": pane.get("agent"), "agent_status": pane.get("agent_status"), "scroll": pane.get("scroll"),
            "pgid==shell": info.get("foreground_process_group_id") == info.get("shell_pid"), "foreground": fg}


def main():
    root = tempfile.mkdtemp(prefix="kampr-muffit-ring-")
    port = free_port()
    config = os.path.join(root, "muffit.toml")
    open(config, "w").write(f'''[provider]
kind = "openai"
model = "test-model"
base_url = "http://127.0.0.1:{free_port()}"
api_key = "test-key"

[server]
host = "127.0.0.1"
port = {port}
token = "probe-token-{os.getpid()}"

[staging]
tier_b = "off"

[storage]
path = "{root}/muffit.db"
''')
    os.makedirs(f"{root}/home", exist_ok=True)
    env = f"HOME={root}/home MUFFIT_CONFIG_FILE={config} MUFFIT_HOME={root}/muffit"
    print("herdr", subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip(), flush=True)
    try:
        with Session("muffit-ring") as s:
            for label, launch in [("muffit", f"{env} muffit"),
                                  ("muffit-tui", f"({env} muffit-server >/dev/null 2>&1 &) ; sleep 3; {env} muffit-tui --server 127.0.0.1:{port}")]:
                p = s.pane(label)
                s.send(p, f"cd {s.cwd} && seq 1 300\r"); time.sleep(5)
                print(f"== {label} before", json.dumps(reading(s, p)), flush=True)
                s.send(p, launch + "\r"); time.sleep(0.5)
                print(f"== {label} t+0.5s", json.dumps(reading(s, p)), flush=True)
                time.sleep(9.5)
                for i in range(3):
                    print(f"== {label} running t+{i*3}s", json.dumps(reading(s, p)), flush=True)
                    time.sleep(3)
                print("   screen:", [r for r in s.screen(p).splitlines() if r.strip()][:4], flush=True)
                s.send(p, "\x03"); time.sleep(0.5); s.send(p, "\x03"); time.sleep(2)
                print(f"== {label} after exit", json.dumps(reading(s, p)), flush=True)
                print("   screen tail:", [r for r in s.screen(p).splitlines() if r.strip()][-3:], flush=True)
    finally:
        subprocess.run(f"{env} muffit server stop", shell=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["pkill", "-f", f"MUFFIT_HOME={root}/muffit"], check=False)
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
