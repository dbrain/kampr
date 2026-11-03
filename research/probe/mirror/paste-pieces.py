import glob, json, os, sys, time
from common import Session, marker

# Usage: paste-pieces.py <text-file> [tail]: types the file as ComposerMirror.pieces does (each line
# its own write, a lone `\n` between, back to back), then `tail` a key at a time, then Enter, and
# prints the box and the record Claude wrote.
text = open(sys.argv[1]).read()
tail = sys.argv[2] if len(sys.argv) > 2 else " rust? any KMP/CMP tv libraries we missed?"

with Session("pieces") as s:
    pane = s.pane()
    s.boot_claude(pane)
    if os.environ.get("ONE_WRITE"):
        s.send(pane, text)
    else:
        for i, piece in enumerate(text.split("\n")):
            if i:
                s.send(pane, "\n")
            if piece:
                s.send(pane, piece)
    time.sleep(1.5)
    print("--- after burst\n" + s.screen(pane))
    for ch in tail:
        s.send(pane, ch)
        time.sleep(0.12)
    time.sleep(1.5)
    print("--- after tail\n" + s.screen(pane))
    s.send(pane, "\r")
    time.sleep(4)
    m = marker(s.cwd)
    sid = m and m.get("sessionId")
    for f in glob.glob(os.path.expanduser(f"~/.claude/projects/*/{sid}.jsonl")):
        for line in open(f):
            r = json.loads(line)
            if r.get("type") == "user" and isinstance(r["message"]["content"], str):
                c = r["message"]["content"]
                print("--- record", len(c), "sent", len(text) + len(tail), "doubled" if c.count(text) > 1 else "single")
                print(repr(c))
