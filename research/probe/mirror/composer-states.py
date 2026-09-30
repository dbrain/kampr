#!/usr/bin/env python3
"""Which of Claude's own composer states does herdr call `blocked`, and what does Claude's session
marker say in each — the two inputs the node's `pending` gate is built from.

Each state is entered from a composer holding a half-typed line under a reply that asks "do you
want to…", sampled, and left with Escape. Throwaway named herdr session; frames under frames/.
"""
import json, os, sys, time
from common import Session, HERE, marker

OUT = os.path.join(HERE, "frames")
PROMPT = ("Reply with exactly these two sentences and nothing else, no tools: "
          "I can keep the old config or replace it. Do you want to keep it, or would you like to replace it?")
STATES = [
    ("esc-once", "\x1b"),
    ("esc-twice", "\x1b\x1b"),
    ("down", "\x1b[B"),
    ("up", "\x1b[A"),
    ("ctrl-r", "\x12"),
    ("shift-tab", "\x1b[Z"),
]


def sample(s, pane, label):
    st, _ = s.status(pane)
    m = marker(s.cwd) or {}
    rule = (s.explain(pane).get("matched_rule") or {}).get("id")
    text = s.screen(pane)
    with open(os.path.join(OUT, f"state-{label}.txt"), "w") as f:
        f.write(text)
    print(f"{label:22s} herdr={st:8s} rule={rule}  marker={m.get('status')}/{m.get('waitingFor')}", flush=True)


def main():
    os.makedirs(OUT, exist_ok=True)
    with Session("states") as s:
        pane = s.pane("claude")
        time.sleep(1)
        s.boot_claude(pane)
        s.send(pane, "first message, say ok")
        time.sleep(0.4); s.send(pane, "\r"); time.sleep(8)
        s.send(pane, PROMPT)
        time.sleep(0.4); s.send(pane, "\r"); time.sleep(10)
        for label, keys in STATES:
            for typed in ("", "push the branch"):
                s.send(pane, typed)
                time.sleep(0.8)
                s.send(pane, keys)
                time.sleep(1.5)
                sample(s, pane, f"{label}{'-typed' if typed else ''}")
                s.send(pane, "\x1b")
                time.sleep(0.8)
                s.send(pane, "\x7f" * 40)
                time.sleep(0.8)


if __name__ == "__main__":
    main()
