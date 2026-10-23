"""What herdr makes of a pane hosting a session it did not spawn (docs/17, probes #575-#586).

    python3 research/probe/native-host.py launch     # #575 typing `exec` into a fresh pane, on the operator's real shell
    python3 research/probe/native-host.py report     # #576 a `kampr` pane given reports: label, sidebar, border, TTL, release
    python3 research/probe/native-host.py fidelity   # #577 herdr's reads of a passthrough pane against a direct one, and resize
    python3 research/probe/native-host.py done       # #578 `done` armed by reports alone
    python3 research/probe/native-host.py lifetime   # #579 #580 what herdr's stop/SIGTERM/SIGKILL take with them, and a holder outside it
    python3 research/probe/native-host.py comm       # #581 PR_SET_NAME as a label
    python3 research/probe/native-host.py resume     # #583 0.9.2's `resume_argv` re-hosting a session after a restart
    python3 research/probe/native-host.py completion # #584 0.9.2's `completion_seq` against `done` and focus
    python3 research/probe/native-host.py handoff    # #585 #586 `herdr update --handoff` bare and under a unit like herdr.service
    python3 research/probe/native-host.py compat     # a target-less split, `env` on a split, and a `resume_argv` led by a path or by `env`

`resume` and `completion` need herdr 0.9.2 or later; point PATH at another herdr to compare.
`handoff` runs a copy of the installed herdr and lets it update itself to the latest release.

Every mode but `launch` runs herdr under an empty HOME, so no shell rc, history or atuin is
involved; `launch` uses the operator's real rc on purpose and keeps its line out of history with
`unset HISTFILE`. Sessions are throwaway and stopped on the way out.
"""

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
import ptyclient
import vt

WORK = tempfile.mkdtemp(prefix="kph-", dir=os.environ.get("XDG_RUNTIME_DIR"))
KAMPR = os.path.join(WORK, "kampr")
os.symlink(sys.executable, KAMPR)
PASS = f"{KAMPR} -I {HERE}/passthrough.py"
HOLD = f"{HERE}/holder.py"
EMPTY_HOME = os.path.join(WORK, "h")
os.makedirs(EMPTY_HOME)
for key in [k for k in os.environ if k.startswith("HERDR_")]:
    del os.environ[key]


class Herdr:
    def __init__(self, name, real_home=False):
        self.name = name
        home = os.path.expanduser("~") if real_home else EMPTY_HOME
        self.env = dict(os.environ) if real_home else {
            k: v for k, v in os.environ.items() if not k.startswith(("ATUIN", "BLE", "XDG_CONFIG", "XDG_STATE", "XDG_DATA"))
        } | {"HOME": home, "HISTFILE": os.path.join(WORK, "hist"), "SHELL": "/usr/bin/bash"}
        self.dir = os.path.join(home, ".config/herdr/sessions", name)
        self.sock = os.path.join(self.dir, "herdr.sock")

    def start(self):
        self.proc = subprocess.Popen(["herdr", "server", "--session", self.name], stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True, env=self.env)
        for _ in range(200):
            if os.path.exists(self.sock):
                time.sleep(0.3)
                return self
            time.sleep(0.1)
        raise SystemExit(f"herdr never opened {self.sock}")

    def stop(self, wipe=True):
        try:
            rpc("server.stop", {}, sock_path=self.sock)
        except OSError:
            pass
        self.proc.wait(5)
        if wipe:
            shutil.rmtree(self.dir, ignore_errors=True)

    def __call__(self, method, **params):
        return rpc(method, params, sock_path=self.sock)

    def ok(self, method, **params):
        return self(method, **params)["result"]

    def pane(self, p):
        return self.ok("pane.get", pane_id=p)["pane"]

    def fg(self, p):
        r = self("pane.process_info", pane_id=p)
        return [x["name"] for x in r["result"]["process_info"]["foreground_processes"]] if "result" in r else r["error"]["code"]

    def text(self, p, source="visible", fmt="text", lines=None):
        q = {"pane_id": p, "source": source, "format": fmt} | ({"lines": lines} if lines else {})
        return self.ok("pane.read", **q)["read"]["text"]

    def type(self, p, line):
        self("pane.send_text", pane_id=p, text=line + "\r")

    def run(self, p, cmd, wait_for="kampr", timeout=15):
        t0 = time.time()
        self.type(p, f" unset HISTFILE; exec {cmd}")
        while time.time() - t0 < timeout:
            f = self.fg(p)
            if f == "pane_not_found":
                return "died"
            if wait_for in f:
                return round(time.time() - t0, 2)
            time.sleep(0.03)
        return None

    def tab(self):
        return self.ok("tab.create", workspace_id="w1")["root_pane"]["pane_id"]

    def desk(self, cols=140, rows=40):
        pid, fd = ptyclient.spawn(["herdr", "--session", self.name], cols, rows, env={"HOME": self.env["HOME"]})
        out = ptyclient.drain(fd, 2.5).decode("utf-8", "replace")
        os.kill(pid, signal.SIGTERM)
        ptyclient.drain(fd, 0.5)
        sc = vt.Screen(cols, rows)
        vt.feed(sc, out)
        return sc.text()


def session(name, real_home=False):
    h = Herdr(f"kph-{name}-{os.getpid()}", real_home).start()
    h.ok("workspace.create")
    time.sleep(1)
    return h


def launch():
    h = session("launch", real_home=True)
    marker = f"KPHLAUNCH{os.getpid()}"
    try:
        time.sleep(2)
        for delay in [0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 0.0, 1.5, 3.0]:
            p = h.ok("pane.split", direction="right", target_pane_id="w1:p1")["pane"]["pane_id"]
            time.sleep(delay)
            print(p, f"typed at +{delay}s ->", h.run(p, f"{PASS} -- sleep 1000 # {marker}"))
            h("pane.close", pane_id=p)
    finally:
        h.stop()
    time.sleep(1)
    leaked = subprocess.run(["grep", "-c", marker, os.path.expanduser("~/.bash_history")], capture_output=True, text=True).stdout.strip()
    print("bash_history lines with the marker:", leaked)


def report():
    h = session("report")
    try:
        p = h.ok("pane.split", direction="right", target_pane_id="w1:p1")["pane"]["pane_id"]
        h.run(p, f"{PASS} -- bash --norc")
        keys = lambda: {k: h.pane(p).get(k) for k in ["agent", "agent_status", "title", "agent_session"]}
        agents = lambda: [(a["pane_id"], a["agent"], a["agent_status"]) for a in h.ok("agent.list")["agents"]]
        print("before any report", keys(), agents())
        h("pane.report_agent", pane_id=p, source="kampr", agent="claude", state="working")
        h("pane.report_metadata", pane_id=p, source="kampr", title="zulu (native s7)")
        h("pane.report_agent_session", pane_id=p, source="kampr", agent="claude", agent_session_id="sess-123",
          agent_session_path="/tmp/x/sess-123.jsonl")
        time.sleep(0.5)
        print("after reports", keys(), agents())
        print("session id anywhere in the snapshot:", "sess-123" in json.dumps(h.ok("session.snapshot")))
        print("\n".join(line for line in h.desk().splitlines() if "zulu" in line or "claude" in line))
        h("pane.report_agent", pane_id=p, source="kampr", agent="claude", state="blocked", message="Allow Bash?")
        for t in range(4):
            time.sleep(5)
            print(f"+{5 * (t + 1)}s", keys()["agent_status"])
        h("pane.release_agent", pane_id=p, source="kampr", agent="claude")
        time.sleep(0.5)
        print("released", keys(), agents())
    finally:
        h.stop()


def fidelity():
    prog = os.path.join(WORK, "prog.sh")
    with open(prog, "w") as f:
        f.write(r"""printf '\033[H\033[2J\033[3J'
echo BEGIN
for i in $(seq 1 300); do printf '\033[38;5;%dmline %03d\033[0m \033[1;48;2;10;20;%dmrgb\033[0m 漢字 é\n' $((i%256)) $i $((i%256)); done
printf '\033]8;;https://example.com/x\033\\a link\033]8;;\033\\ after\n'
printf '%0.sw' $(seq 1 130); echo
printf '\033[?1049h\033[H\033[2JALT SCREEN\033[5;5Hmid\033[?1000h'; sleep 0.5; printf '\033[?1000l\033[?1049l'
echo AFTER-ALT
printf 'no newline at end'
""")
    winch = os.path.join(WORK, "winch.sh")
    with open(winch, "w") as f:
        f.write('stty size\ntrap "stty size" WINCH\nwhile :; do sleep 0.1; done\n')
    h = session("fidelity")
    try:
        print("first pane of a workspace vs a tab's root pane, `stty size`:")
        b = h.tab()
        time.sleep(1)
        for p in ["w1:p1", b]:
            h.type(p, " stty size")
        time.sleep(1)
        print("   ", [h.text(p).strip().splitlines()[-2] for p in ["w1:p1", b]])
        direct, passed = h.tab(), h.tab()
        time.sleep(1)
        h.run(direct, f'bash --norc -c "bash {prog}; sleep 1000"', wait_for="bash")
        h.run(passed, f'{PASS} -- bash --norc -c "bash {prog}; sleep 1000"')
        time.sleep(2.5)
        for source, fmt, lines in [("visible", "ansi", None), ("recent", "ansi", 2000), ("recent_unwrapped", "text", 2000)]:
            a, z = (h.text(p, source, fmt, lines) for p in (direct, passed))
            if source != "visible":
                a, z = a[a.index("BEGIN"):], z[z.index("BEGIN"):]
            print(source, fmt, "identical" if a == z else "DIFFER", len(a), len(z))
        print("scroll", h.pane(direct)["scroll"], h.pane(passed)["scroll"])
        p, q = h.tab(), None
        q = h.ok("pane.split", direction="right", target_pane_id=p)["pane"]["pane_id"]
        time.sleep(1)
        h.run(p, f"bash --norc {winch}", wait_for="bash")
        h.run(q, f"{PASS} -- bash --norc {winch}")
        time.sleep(1)
        h.ok("tab.focus", tab_id=h.pane(p)["tab_id"])
        size = lambda: [" | ".join(h.text(x).strip().splitlines()[-3:]) for x in (p, q)]
        print("headless, after a split", size())
        pid, fd = ptyclient.spawn(["herdr", "--session", h.name], 160, 50, env={"HOME": EMPTY_HOME})
        ptyclient.drain(fd, 2.5)
        print("desk at 160x50          ", size())
        import fcntl, struct, termios
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        os.kill(pid, signal.SIGWINCH)
        ptyclient.drain(fd, 2)
        print("desk at 100x30          ", size())
        os.kill(pid, signal.SIGTERM)
        ptyclient.drain(fd, 1)
    finally:
        h.stop()


def done():
    for gap, focused in [(0, False), (0.3, False), (2, False), (10, False), (2, True), (2, False)]:
        h = Herdr(f"kph-done-{os.getpid()}").start()
        try:
            h.ok("workspace.create")
            h.ok("workspace.create")
            time.sleep(1)
            p = "w1:p1" if focused else "w2:p1"
            if focused:
                h.ok("workspace.focus", workspace_id="w1")
            h.run(p, f"{PASS} -- sleep 1000")
            h("pane.report_agent", pane_id=p, source="kampr", agent="claude", state="working")
            time.sleep(gap)
            h("pane.report_agent", pane_id=p, source="kampr", agent="claude", state="idle")
            seen = []
            for _ in range(40):
                seen.append(h.pane(p)["agent_status"])
                time.sleep(0.1)
            h.ok("pane.focus", pane_id=p)
            time.sleep(0.3)
            print(f"gap {gap}s", "focused  " if focused else "unfocused", sorted(set(seen)), "-> after pane.focus", h.pane(p)["agent_status"])
        finally:
            h.stop()


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().split()[2] != "Z"
    except OSError:
        return False


def kill(h, how):
    if how == "stop":
        h("server.stop")
    else:
        h.proc.send_signal(signal.SIGKILL if how == "sigkill" else signal.SIGTERM)
    h.proc.wait(5)
    time.sleep(1)


def lifetime():
    for how in ["stop", "sigterm", "sigkill"]:
        h = session(f"life-{how}")
        pidfile = os.path.join(WORK, f"pt-{how}")
        h.run("w1:p1", f"{PASS} --pidfile {pidfile} -- sleep 1000")
        time.sleep(0.5)
        outer, inner = map(int, open(pidfile).read().split())
        kill(h, how)
        print(f"herdr {how}: pane process alive", alive(outer), "| program on its pty alive", alive(inner))
        h.start()
        print("    restarted:", [(p["pane_id"], h.fg(p["pane_id"])) for p in h.ok("pane.list")["panes"]])
        h.stop()
    sock, pidfile = os.path.join(WORK, "hold.sock"), os.path.join(WORK, "hold.pid")
    subprocess.run([sys.executable, "-I", HOLD, "serve", sock, "--pidfile", pidfile, "--", "bash", "--norc"])
    time.sleep(0.5)
    hold, prog = map(int, open(pidfile).read().split())
    h = Herdr(f"kph-hold-{os.getpid()}")
    try:
        for round_, how in enumerate(["sigkill", "stop", "sigterm"]):
            h.start()
            if not h.ok("pane.list")["panes"]:
                h.ok("workspace.create")
                time.sleep(1)
            p = h.ok("pane.list")["panes"][0]["pane_id"]
            h.run(p, f"{KAMPR} -I {HOLD} attach {sock}")
            h.type(p, f"echo MARK{round_}")
            time.sleep(0.6)
            seen = h.text(p, "recent", "text", 200).replace("echo MARK", "")
            print(f"attached into {p}: history holds", [m for m in ("MARK0", "MARK1", "MARK2") if m in seen])
            if round_ == 0:
                q = h.ok("pane.split", direction="right", target_pane_id=p)["pane"]["pane_id"]
                time.sleep(1)
                h.run(q, f"{KAMPR} -I {HOLD} attach {sock}")
                h.type(q, "echo FROM-SECOND")
                time.sleep(0.6)
                print("    a second attached pane's input reaches the first:", "FROM-SECOND" in h.text(p).replace("echo FROM", ""))
                h("pane.close", pane_id=q)
                time.sleep(0.5)
            kill(h, how)
            print(f"    herdr {how}: holder alive", alive(hold), "| program alive", alive(prog))
    finally:
        shutil.rmtree(h.dir, ignore_errors=True)
        for pid in (prog, hold):
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass


def resume():
    os.environ["PATH"] = WORK + ":" + os.environ["PATH"]
    sock, pidfile = os.path.join(WORK, "hold.sock"), os.path.join(WORK, "hold.pid")
    subprocess.run([sys.executable, "-I", HOLD, "serve", sock, "--pidfile", pidfile, "--", "bash", "--norc"])
    time.sleep(0.5)
    hold, prog = map(int, open(pidfile).read().split())
    h = Herdr(f"kph-resume-{os.getpid()}")
    h.env["PATH"] = os.environ["PATH"]
    try:
        h.start()
        h.ok("workspace.create")
        time.sleep(1)
        argv = ["kampr", "-I", HOLD, "attach", sock]
        h.run("w1:p1", " ".join(argv))
        h.type("w1:p1", "echo BEFORE-RESTART")
        time.sleep(0.5)
        print("report_agent with resume_argv ->", h("pane.report_agent", pane_id="w1:p1", source="kampr", agent="claude", state="idle", resume_argv=argv))
        time.sleep(0.5)
        for how in ["stop", "sigkill"]:
            kill(h, how)
            print(f"herdr {how}: holder alive", alive(hold), "| program alive", alive(prog))
            h.start()
            time.sleep(4)
            for x in h.ok("pane.list")["panes"]:
                seen = h.text(x["pane_id"], "recent", "text", 200).replace("echo BEFORE", "")
                print("    restarted", x["pane_id"], "fg", h.fg(x["pane_id"]), "agent", x.get("agent"), x["agent_status"],
                      "restore_error", x.get("restore_error"), "| replayed history:", "BEFORE-RESTART" in seen)
    finally:
        h.stop()
        for pid in (prog, hold):
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass


def completion():
    h = Herdr(f"kph-completion-{os.getpid()}").start()
    try:
        h.ok("workspace.create")
        h.ok("workspace.create")
        time.sleep(1)
        p = "w2:p1"
        h.run(p, f"{PASS} -- sleep 1000")
        info = lambda: next(a for a in h.ok("agent.list")["agents"] if a["pane_id"] == p)
        show = lambda tag: print(f"{tag:<24}", info()["agent_status"], "completion_seq", info().get("completion_seq"), "state_change_seq", info().get("state_change_seq"))
        rep = lambda s: (h("pane.report_agent", pane_id=p, source="kampr", agent="claude", state=s), time.sleep(0.3))
        rep("working"); show("working, unfocused")
        rep("idle"); show("idle, unfocused")
        h.ok("pane.focus", pane_id=p); time.sleep(0.3); show("after pane.focus")
        rep("working"); rep("idle"); show("second turn, focused")
        rep("working"); show("third turn, working")
        for state in ["working", "idle", "blocked"]:
            h("pane.report_agent", pane_id="w1:p1", source="kampr", agent="claude", state=state)
            seen = set()
            for _ in range(30):
                pane = h.pane("w1:p1")
                seen.add((pane.get("agent"), pane["agent_status"]))
                time.sleep(0.2)
            print(f"`{state}` reported on an idle bash pane, 6 s:", sorted(seen))
    finally:
        h.stop()


def comm():
    h = session("comm")
    try:
        renamed, named = h.tab(), h.tab()
        time.sleep(1)
        bin_claude = os.path.join(WORK, "bin", "claude")
        os.makedirs(os.path.dirname(bin_claude))
        os.symlink(sys.executable, bin_claude)
        h.run(renamed, f"{PASS} --comm claude -- sleep 1000", wait_for="claude")
        h.run(named, f"{bin_claude} -I {HERE}/passthrough.py -- sleep 1000", wait_for="claude")
        h.run("w1:p1", f"{PASS} -- sleep 1000")
        for _ in range(3):
            time.sleep(3)
            print({p: (h.pane(p).get("agent"), h.pane(p)["agent_status"]) for p in (renamed, named, "w1:p1")})
        rules = h.ok("agent.explain", target=renamed)["explain"]["evaluated_rules"]
        print("rules herdr evaluated over the renamed pane:", len(rules))
    finally:
        h.stop()


def compat():
    os.environ["PATH"] = WORK + ":" + os.environ["PATH"]
    rec = os.path.join(WORK, "rec.py")
    out = os.path.join(WORK, "rec")
    with open(rec, "w") as f:
        f.write("import os,sys,time\nopen(sys.argv[1],'a').write(repr([os.environ.get(k) for k in "
                "('KPROBE','HERDR_PANE_ID','HERDR_SOCKET_PATH')])+'\\n')\ntime.sleep(1000)\n")
    h = Herdr(f"kph-compat-{os.getpid()}")
    h.env["PATH"] = os.environ["PATH"]
    try:
        h.start()
        h.ok("workspace.create")
        h.ok("workspace.create")
        time.sleep(1)
        before = h.ok("session.snapshot")["snapshot"]
        print("focused before", before.get("focused_workspace_id"), [(p["pane_id"], p.get("focused")) for p in before["panes"]])
        p = h.ok("pane.split", direction="right", focus=False, env={"KPROBE": "from-split"})["pane"]["pane_id"]
        after = h.ok("session.snapshot")["snapshot"]
        print("split with no target made", p, "| focused after", after.get("focused_workspace_id"), [(x["pane_id"], x.get("focused")) for x in after["panes"]])
        h.run(p, f"{KAMPR} -I {rec} {out}")
        time.sleep(0.5)
        for argv in ([KAMPR, "-I", rec, out], ["kampr", "-I", rec, out], ["env", KAMPR, "-I", rec, out]):
            print("resume_argv", argv[0], "->", h("pane.report_agent", pane_id=p, source="kampr", agent="kampr", state="idle", resume_argv=argv))
        for how in ["stop", "sigkill"]:
            kill(h, how)
            h.start()
            time.sleep(4)
            print(f"after {how}:", p, "fg", h.fg(p))
        print("env seen at each start [KPROBE, HERDR_PANE_ID, HERDR_SOCKET_PATH]:", open(out).read().splitlines())
    finally:
        h.stop()
        subprocess.run(["pkill", "-f", rec])


def handoff():
    installed = shutil.which("herdr")
    for under_unit in (False, True):
        bindir = os.path.join(WORK, f"bin{int(under_unit)}")
        os.makedirs(bindir)
        herdr = os.path.join(bindir, "herdr")
        shutil.copy2(installed, herdr)
        version = lambda: subprocess.run([herdr, "--version"], capture_output=True, text=True).stdout.strip()
        h = Herdr(f"kph-handoff-{int(under_unit)}-{os.getpid()}")
        h.env["PATH"] = bindir + ":" + h.env["PATH"]
        unit = ["systemctl", "--user", "show", h.name, "-p", "ActiveState,NRestarts"]
        sleep = None
        try:
            if under_unit:
                subprocess.run(["systemd-run", "--user", "-q", "--unit", h.name, "-p", "Type=simple", "-p", "Restart=on-failure",
                                "-p", "RestartSec=5", f"--setenv=HOME={h.env['HOME']}", "--setenv=SHELL=/usr/bin/bash",
                                herdr, "server", "--session", h.name], check=True)
                while not os.path.exists(h.sock):
                    time.sleep(0.1)
                time.sleep(0.5)
            else:
                h.start()
            h.ok("workspace.create")
            h.ok("workspace.create")
            time.sleep(1)
            h.run("w1:p1", "sleep 1001", wait_for="sleep")
            h.type("w2:p1", "echo HISTORY-MARK")
            h("pane.report_agent", pane_id="w2:p1", source="kampr", agent="claude", state="working")
            time.sleep(0.5)
            sleep = next(x["pid"] for x in h.ok("pane.process_info", pane_id="w1:p1")["process_info"]["foreground_processes"])
            print(f"=== {'under a unit' if under_unit else 'bare'}: {version()}")
            r = subprocess.run([herdr, "--session", h.name, "update", "--handoff"], env=h.env, capture_output=True, text=True)
            print("   ", r.stderr.strip().splitlines()[-2])
            time.sleep(15 if under_unit else 3)
            print("   ", version(), "| sleep alive", alive(sleep), "| socket", os.path.exists(h.sock),
                  *(subprocess.run(unit, capture_output=True, text=True).stdout.split() if under_unit else []))
            if os.path.exists(h.sock):
                print("    panes", [(p["pane_id"], p.get("agent"), p["agent_status"]) for p in h.ok("pane.list")["panes"]],
                      "| w1:p1 fg", h.fg("w1:p1"), "| history kept", "HISTORY-MARK" in h.text("w2:p1", "recent", "text", 200).replace("echo HISTORY", ""))
        finally:
            if os.path.exists(h.sock):
                h("server.stop")
            subprocess.run(["systemctl", "--user", "stop", h.name], stderr=subprocess.DEVNULL)
            subprocess.run(["systemctl", "--user", "reset-failed", h.name], stderr=subprocess.DEVNULL)
            if sleep and alive(sleep):
                os.kill(sleep, signal.SIGKILL)
            shutil.rmtree(h.dir, ignore_errors=True)


if __name__ == "__main__":
    try:
        {"launch": launch, "report": report, "fidelity": fidelity, "done": done, "lifetime": lifetime, "comm": comm, "resume": resume, "completion": completion, "handoff": handoff, "compat": compat}[sys.argv[1]]()
    finally:
        shutil.rmtree(WORK, ignore_errors=True)
