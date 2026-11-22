import json, os, subprocess, sys, time
from importlib import import_module

# Usage: clipped-box.py <out-dir> [claude|codex|agy ...]. A message taller than the harness's box can
# show, typed at the node's pace (#666), captured as a composer fixture (`caret <col> <row>` then
# the grid, from Kampr's own emulator through `composer_look`) at 93x40, with the message beside it
# and what the reader made of each.
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
cl = import_module("composer-line")
cl.COLS, cl.ROWS = 93, 40
BOOT = {"claude": ("claude --model haiku", 12, []), "codex": ("codex", 12, [("\r", 6)]), "agy": ("agy", 16, [("\r", 10)])}
LOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../target/debug/examples/composer_look")
WORDS = "the quick brown fox jumps over the lazy dog while the kettle boils "

out = sys.argv[1]
text = "\n".join((f"{i}:" + WORDS * 12)[:700].rstrip() for i in range(6))
cl.start()
try:
    for agent in sys.argv[2:] or ["claude", "codex"]:
        cwd = os.popen("mktemp -d /tmp/kampr-clip-XXXXXX").read().strip()
        pane = cl.call("workspace.create", {"label": agent, "cwd": cwd})["root_pane"]["pane_id"]
        control = subprocess.Popen(["herdr", "terminal", "session", "control", pane, "--cols", "93", "--rows", "40"],
                                   stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   env=dict(os.environ, HERDR_SOCKET_PATH=cl.SOCK))
        time.sleep(1)
        look = subprocess.Popen([LOOK, cl.SOCK, pane, "93", "40", agent], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, text=True, env=dict(os.environ, HERDR_SOCKET_PATH=cl.SOCK))
        cmd, boot, keys = BOOT[agent]
        time.sleep(1)
        cl.send(pane, cmd + "\r")
        time.sleep(boot)
        for k, pause in keys:
            cl.send(pane, k)
            time.sleep(pause)
        for i, line in enumerate(text.split("\n")):
            if i:
                cl.send(pane, "\n")
                time.sleep(0.01)
            for at in range(0, len(line), 400):
                cl.send(pane, line[at:at + 400])
                time.sleep(len(line[at:at + 400]) / 2000)
        time.sleep(8)
        look.stdin.write("look\n")
        look.stdin.flush()
        got = json.loads(look.stdout.readline())
        (x, y), rows = got["caret"], [r.rstrip() for r in got["rows"]]
        with open(os.path.join(out, f"{agent}-clipped.txt"), "w") as f:
            f.write(f"caret {x} {y}\n" + "\n".join(rows) + "\n")
        print(agent, "read", len(got["read"] or ""), "suffix" if got["read"] and text.endswith(got["read"]) else repr((got["read"] or "")[:120]))
        look.kill()
        control.kill()
    with open(os.path.join(out, "clipped-typed.txt"), "w") as f:
        f.write(text)
finally:
    cl.stop()
