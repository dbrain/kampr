import glob, json, os, time
from common import Session, marker

# Usage: paste-long.py. Whether a message longer than one 800-character write (#564) can be typed
# into Claude's box as plain text: each trial writes a multi-line message as ComposerMirror.pieces
# does (each line its own write, a lone `\n` between) with every line cut into writes of at most
# `chunk` characters and `gap` seconds between writes (a negative gap is a rate: each write waits its length over it), then Enter 0.4 s later, and reads the
# record Claude wrote: the message verbatim, or wrapped as `<pasted_content>` (#665).
LINE = "the quick brown fox jumps over the lazy dog while the kettle boils and the cat watches on "


def message(n, lines):
    per = n // lines
    body = [(f"{i}:" + LINE * (per // len(LINE) + 1))[:per].rstrip() for i in range(lines)]
    return "Reply with only the word ok. " + "\n".join(body)


def writes(text, chunk):
    out = []
    for i, line in enumerate(text.split("\n")):
        if i:
            out.append("\n")
        out += [line[at:at + chunk] for at in range(0, len(line), chunk)]
    return out


TRIALS = [
    (870, 5, 800, 0.0),
    (1500, 5, 800, 0.0),
    (3000, 6, 800, 0.0),
    (1500, 1, 800, 0.0),
    (1500, 1, 400, 0.0),
    (1500, 1, 400, 0.05),
    (3000, 1, 400, 0.05),
    (3000, 6, 400, 0.05),
    (6000, 8, 400, 0.05),
    (1500, 1, 400, -2000),
    (6000, 8, 400, -2000),
    (6000, 3, 400, -2000),
]

with Session("long") as s:
    pane = s.pane()
    s.boot_claude(pane)
    m = marker(s.cwd)
    sid = m and m.get("sessionId")
    seen = 0
    for n, lines, chunk, gap in TRIALS:
        text = message(n, lines)
        for w in writes(text, chunk):
            s.send(pane, w)
            time.sleep(len(w) / -gap if gap < 0 else gap)
        time.sleep(0.4)
        s.send(pane, "\r")
        record = None
        for _ in range(60):
            time.sleep(0.5)
            got = []
            for f in glob.glob(os.path.expanduser(f"~/.claude/projects/*/{sid}.jsonl")):
                for line in open(f):
                    r = json.loads(line)
                    if r.get("type") == "user" and not r.get("isMeta"):
                        c = r["message"]["content"]
                        c = c if isinstance(c, str) else "".join(b.get("text", "") for b in c if isinstance(b, dict))
                        if c.startswith("Reply with only") or "<pasted_content" in c:
                            got.append(c)
            if len(got) > seen:
                record = got[seen]
                seen = len(got)
                break
        verdict = "none" if record is None else "verbatim" if record == text else "pasted" if "<pasted_content" in record else f"other({len(record)})"
        print(f"len={len(text)} lines={lines} chunk={chunk} gap={gap}: {verdict}", flush=True)
        if record is None:
            s.send(pane, "\x03")
        time.sleep(2)
        s.wait_status(pane, "idle")
