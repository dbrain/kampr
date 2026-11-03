#!/usr/bin/env python3
"""Claude's box holding a reply of two paragraphs, captured in `composer-line.py`'s fixture format:
`caret <col> <row>` then the grid, folded from a real `observe` stream. Writes the capture to the
path given, for `crates/kampr-journal/tests/fixtures/composer`.

Usage: composer-paragraphs.py <out.txt>
"""
import importlib.util, os, subprocess, shutil, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("composer_line", os.path.join(HERE, "composer-line.py"))
line = importlib.util.module_from_spec(spec)
spec.loader.exec_module(line)
line.COLS = 93

FIRST = "push the branch when the tests go green"
SECOND = "then tag it"

cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-paragraphs-XXXXXX"], capture_output=True, text=True).stdout.strip()
line.start()
try:
    ws = line.call("workspace.create", {"label": "claude", "cwd": cwd})
    pane = ws["root_pane"]["pane_id"]
    watch = line.Watch(pane)
    time.sleep(1.0)
    line.send(pane, "claude\r")
    for _ in range(60):
        time.sleep(0.5)
        rows, _, _ = watch.look()
        if any("trust this folder" in r for r in rows):
            line.send(pane, "\r")
            time.sleep(4)
            break
        if any(r.startswith("❯") for r in rows):
            time.sleep(2)
            break
    for piece in (FIRST, "\n", "\n", SECOND):
        line.send(pane, piece)
        time.sleep(0.3)
    time.sleep(1.5)
    rows, x, y = watch.look()
    with open(sys.argv[1], "w") as f:
        f.write(f"caret {x} {y}\n" + "\n".join(rows) + "\n")
    print(f"caret {x} {y}")
    for i in range(max(0, y - 6), min(len(rows), y + 3)):
        print(f"[{i}]{'*' if i == y else ' '}{rows[i]!r}")
    watch.close()
finally:
    line.stop()
    shutil.rmtree(cwd, ignore_errors=True)
