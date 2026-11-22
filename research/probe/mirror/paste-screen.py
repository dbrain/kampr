import os, sys, time
from common import Session

# Usage: paste-screen.py [claude|codex|agy ...]. The screen half of paste-long.py, for harnesses
# whose records it cannot read: each trial boots the harness in a pane of its own, writes a
# multi-line message as pieces of at most `chunk` characters with `gap` seconds between writes,
# and reports whether the box drew a paste placeholder or the words.
BOOT = {
    "claude": ("claude --model haiku", 12, []),
    "codex": ("codex", 12, [("\r", 6)]),
    "agy": ("agy", 16, [("\r", 10)]),
}
LINE = "the quick brown fox jumps over the lazy dog while the kettle boils and the cat watches on "


def message(n, lines):
    per = n // lines
    return "\n".join((f"{i}:" + LINE * (per // len(LINE) + 1))[:per].rstrip() for i in range(lines))


def writes(text, chunk):
    out = []
    for i, line in enumerate(text.split("\n")):
        if i:
            out.append("\n")
        out += [line[at:at + chunk] for at in range(0, len(line), chunk)]
    return out


TRIALS = [tuple(map(float, t.split(","))) for t in os.environ["TRIALS"].split()] if os.environ.get("TRIALS") else [(1500, 5, 1500, 0.0), (1500, 5, 400, 0.0), (1500, 5, 400, 0.05), (3000, 6, 400, 0.05), (6000, 8, 400, 0.05)]

for agent in sys.argv[1:] or ["codex", "agy"]:
    cmd, wait, keys = BOOT[agent]
    for n, lines, chunk, gap in [(int(a), int(b), int(c), d) for a, b, c, d in TRIALS]:
        with Session(f"pscr-{agent}") as s:
            pane = s.pane()
            s.send(pane, cmd + "\r")
            time.sleep(wait)
            for key, pause in keys:
                s.send(pane, key)
                time.sleep(pause)
            text = message(n, lines)
            for w in writes(text, chunk):
                s.send(pane, w)
                time.sleep(len(w) / -gap if gap < 0 else gap)
            time.sleep(2)
            screen = s.screen(pane)
            pasted = "pasted" in screen.lower()
            last = text.rsplit("\n", 1)[-1][-30:]
            print(f"{agent} len={len(text)} lines={lines} chunk={chunk} gap={gap}: "
                  f"{'placeholder' if pasted else 'words' if last in screen.replace(chr(10), '') else 'neither'}", flush=True)
            if os.environ.get("SHOW"):
                print(screen)
