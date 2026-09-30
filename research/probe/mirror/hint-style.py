#!/usr/bin/env python3
"""How each harness paints the words that sit after the caret in an empty box — Claude's `Try "…"`
hint and its suggested next prompt, Codex's and agy's placeholder — against the operator's own
words with the caret moved to the front by ctrl+a. Throwaway named herdr session; the composer row
is printed with its escapes.

Usage: hint-style.py [claude|codex|agy ...]
"""
import sys, time
from common import Session

MARK = {"claude": "❯", "codex": "›", "agy": ">"}
BOOT = {"claude": ("claude --model haiku", 12, []), "codex": ("codex", 12, [("\r", 6)]),
        "agy": ("agy", 16, [("\r", 10)])}


def row(s, pane, mark):
    text = s.call("pane.read", {"pane_id": pane, "source": "visible", "format": "ansi"})["read"]["text"]
    for line in text.splitlines()[::-1]:
        plain = __import__("re").sub(r"\x1b\[[0-9;?:]*[ -/]*[@-~]", "", line).lstrip()
        if plain.startswith(mark):
            return repr(line)
    return None


def main():
    with Session("hint") as s:
        for name in sys.argv[1:] or ["claude", "codex", "agy"]:
            pane = s.pane(name)
            time.sleep(1)
            cmd, boot, extra = BOOT[name]
            if name == "claude":
                s.boot_claude(pane)
            else:
                s.send(pane, cmd + "\r"); time.sleep(boot)
                for k, p in extra:
                    s.send(pane, k); time.sleep(p)
            print(f"== {name}\n hint      {row(s, pane, MARK[name])}")
            if name == "claude":
                s.send(pane, "say only the word ok"); time.sleep(0.4); s.send(pane, "\r"); time.sleep(12)
                print(f" suggested {row(s, pane, MARK[name])}")
            s.send(pane, "push the branch"); time.sleep(1)
            print(f" typed     {row(s, pane, MARK[name])}")
            s.send(pane, "\x01"); time.sleep(1)
            print(f" ctrl+a    {row(s, pane, MARK[name])}")
            s.send(pane, "\x05" + "\x7f" * 30); time.sleep(1)


if __name__ == "__main__":
    main()
