#!/usr/bin/env python3
"""Replays a byte log written by claude-render.py through an emulator that keeps scrollback, and
counts the rows each marker of the scripted reply lands on (#587, #588).

    python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte
    /tmp/v/bin/python research/probe/claude-replay.py claude-default-turn.bin

The replay is at the pty's starting size throughout, so a log that spans a resize is only
indicative past it.
"""

import sys

import pyte

COLS, ROWS = 100, 30
MARKERS = ["PARA1START", "PARA2START", "Heading for the probe", "bullet 1 ", "code line 1:", "REPLYEND", "LONGANSWER", "Short answer"]

screen = pyte.HistoryScreen(COLS, ROWS, history=100000, ratio=1)
pyte.ByteStream(screen).feed(open(sys.argv[1], "rb").read())
history = ["".join(line[x].data for x in range(COLS)).rstrip() for line in screen.history.top]
rows = history + [l.rstrip() for l in screen.display]
print(f"history rows {len(history)}, visible {ROWS}")
for key in MARKERS:
    print(f"  {key!r}: {sum(key in r for r in rows)} rows, {sum(key in r for r in history)} of them in history")
