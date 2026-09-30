import json, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from rpc import rpc

CLEAN_ENV = {k: v for k, v in os.environ.items()
             if not k.startswith("CLAUDE") and k not in (
                 "AI_AGENT", "HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_ENV",
                 "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_CLIENT_SOCKET_PATH", "HERDR_BIN_PATH",
                 "TERM_PROGRAM")}


class Session:
    def __init__(self, tag):
        self.name = f"kampr-probe-{tag}-{os.getpid()}"
        home = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
        self.sock = os.path.join(home, "herdr", "sessions", self.name, "herdr.sock")
        self.cwd = subprocess.run(["mktemp", "-d", f"/tmp/kampr-{tag}-XXXXXX"],
                                  capture_output=True, text=True).stdout.strip()

    def __enter__(self):
        subprocess.Popen(["herdr", "server", "--session", self.name], env=CLEAN_ENV,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(200):
            if os.path.exists(self.sock):
                time.sleep(0.8)
                return self
            time.sleep(0.1)
        raise SystemExit("herdr never came up")

    def __exit__(self, *exc):
        try:
            rpc("server.stop", {}, sock_path=self.sock)
        except Exception:
            pass
        time.sleep(1.0)
        shutil.rmtree(os.path.dirname(self.sock), ignore_errors=True)
        shutil.rmtree(self.cwd, ignore_errors=True)

    def call(self, method, params=None):
        r = rpc(method, params or {}, sock_path=self.sock)
        if not r or "error" in r:
            raise SystemExit(f"{method} failed: {r}")
        return r["result"]

    def pane(self, label="probe"):
        ws = self.call("workspace.create", {"label": label, "cwd": self.cwd, "focus": False})
        return ws["root_pane"]["pane_id"]

    def send(self, pane, text):
        self.call("pane.send_text", {"pane_id": pane, "text": text})

    def screen(self, pane):
        return self.call("pane.read", {"pane_id": pane, "source": "visible",
                                       "format": "text", "strip_ansi": True})["read"]["text"]

    def status(self, pane):
        for p in self.call("pane.list", {}).get("panes", []):
            if p.get("pane_id") == pane:
                return p.get("agent_status"), p.get("agent")
        return None, None

    def explain(self, pane):
        env = dict(CLEAN_ENV, HERDR_SOCKET_PATH=self.sock)
        out = subprocess.run(["herdr", "agent", "explain", pane, "--json"], env=env,
                             capture_output=True, text=True)
        try:
            return json.loads(out.stdout)
        except ValueError:
            return {"raw": out.stdout + out.stderr}

    def boot_claude(self, pane, args="--model haiku"):
        self.send(pane, f"claude {args}\r")
        for _ in range(120):
            time.sleep(0.5)
            s = self.screen(pane)
            if "trust this folder" in s:
                self.send(pane, "\x1b[B")
                time.sleep(0.4)
                self.send(pane, "\r")
                time.sleep(3)
                continue
            if any(l.startswith("❯") for l in s.splitlines()) and "trust" not in s:
                time.sleep(2)
                return
        raise SystemExit("claude never drew its composer")

    def wait_status(self, pane, want, seconds=90):
        t0 = time.time()
        while time.time() - t0 < seconds:
            if self.status(pane)[0] == want:
                return True
            time.sleep(0.3)
        return False


def marker(cwd):
    root = os.path.expanduser("~/.claude/sessions")
    for name in os.listdir(root):
        if not name.endswith(".json"):
            continue
        try:
            with open(os.path.join(root, name)) as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue
        if d.get("cwd") == cwd:
            return d
    return None
