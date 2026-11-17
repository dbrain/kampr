#!/usr/bin/env python3
"""What each frame of Claude's composer looks like while prose is typed into it faster than it
draws: where each typed character is, and where the caret is, frame by frame.

Types a sentence with spaces into a real `claude` in a throwaway named herdr session, at a slow
gap and then at a fast one, and prints every observed frame's caret and caret row. A guess drawn
ahead of the echo is right only if every frame agrees with one cell per key from the caret.

With `DUMP=<file>` it also writes the whole screen and caret after every key, one JSON object a
line, which is what a composer reader can be run over. With `TEXTS=<file>` as well, each line of
that file is typed in turn, the screen dumped once it has settled, and the box cleared.

Usage: composer-prose.py [COLS] [GAP_MS ...]
"""
import base64, json, os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from importlib import import_module

composer = import_module("composer-line")
import vt

for key in [k for k in os.environ if k.startswith("CLAUDE") or k == "AI_AGENT"]:
    del os.environ[key]

TEXT = os.environ.get("TEXT") or "the quick brown fox jumps over the lazy dog and then it does it all over again, twice"
DUMP = os.environ.get("DUMP")
TEXTS = os.environ.get("TEXTS")


class Frames(composer.Watch):
    def __init__(self, pane):
        self.log = []
        self.t0 = time.time()
        super().__init__(pane)

    def pump(self):
        for line in self.child.stdout:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if rec.get("type") != "terminal.frame":
                continue
            data = base64.b64decode(rec["bytes"]).decode("utf-8", "replace")
            with self.lock:
                vt.feed(self.screen, data)
                s = self.screen
                self.log.append((time.time() - self.t0, s.x, s.y, "".join(s.g[s.y]).rstrip(), "".join(s.g[s.y - 1]).rstrip(), repr(data[:60]) if len(data) < 60 else len(data)))


def run(cols, gaps):
    composer.NAME = f"kampr-probe-prose-{os.getpid()}-{cols}"
    home = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
    composer.SOCK = os.path.join(home, "herdr", "sessions", composer.NAME, "herdr.sock")
    composer.COLS = cols
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-prose-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = composer.call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        env = dict(os.environ, HERDR_SOCKET_PATH=composer.SOCK)
        control = subprocess.Popen(
            ["herdr", "terminal", "session", "control", pane, "--cols", str(cols), "--rows", str(composer.ROWS)],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
        time.sleep(0.5)
        watch = Frames(pane)
        time.sleep(1.0)
        composer.send(pane, "claude --model haiku\r")
        for _ in range(80):
            time.sleep(0.5)
            rows = watch.look()[0]
            if any("trust" in r.lower() for r in rows):
                composer.send(pane, "\r")
                time.sleep(4)
                break
            if any(r.startswith("❯") for r in rows):
                break
        time.sleep(3)
        for text in open(TEXTS).read().splitlines() if TEXTS else []:
            for ch in text:
                composer.send(pane, ch)
                time.sleep(0.004)
            time.sleep(0.7)
            with watch.lock, open(DUMP, "a") as out:
                rows = ["".join(r).rstrip() for r in watch.screen.g]
                out.write(json.dumps({"cols": cols, "typed": text, "caret": [watch.screen.x, watch.screen.y], "rows": rows}, ensure_ascii=False) + "\n")
            composer.send(pane, "\x03")
            time.sleep(0.6)
        for gap in [] if TEXTS else gaps:
            with watch.lock:
                watch.log.clear()
            watch.t0 = time.time()
            sent = []
            out = open(DUMP, "a") if DUMP else None
            for ch in TEXT:
                sent.append((time.time() - watch.t0, ch))
                composer.send(pane, ch)
                time.sleep(gap / 1000)
                if out:
                    with watch.lock:
                        rows = ["".join(r).rstrip() for r in watch.screen.g]
                        out.write(json.dumps({"cols": cols, "typed": "".join(c for _, c in sent), "caret": [watch.screen.x, watch.screen.y], "rows": rows}, ensure_ascii=False) + "\n")
            if out:
                out.close()
            time.sleep(1.5)
            print(f"===== {cols} cols, {gap} ms between keys, {len(TEXT)} keys sent over {sent[-1][0]:.2f}s")
            with watch.lock:
                log = list(watch.log)
            for t, x, y, row, above, data in log:
                print(f"  {t:6.3f} caret {x:3d},{y:2d} |{row}|" + (f"  above |{above[-30:]}|" if above.strip() and not above.startswith("─") else "") + f"  {data}")
            composer.send(pane, "\x03")
            time.sleep(1.0)
        watch.close()
        control.kill()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    args = [int(a) for a in sys.argv[1:]]
    run(args[0] if args else 95, args[1:] or [150, 15])
