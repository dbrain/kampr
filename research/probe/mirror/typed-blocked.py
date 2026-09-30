#!/usr/bin/env python3
"""Does typing into Claude's composer, under a reply that asks "do you want to…", flip herdr's
agent_status to `blocked` — and what does the pending reader then see on the screen?

Throwaway named herdr session, torn down. Saves each screen under frames/.
"""
import json, os, sys, time
from common import Session, HERE, marker

OUT = os.path.join(HERE, "frames")
PROMPT = ("Reply with exactly these two sentences and nothing else, no tools: "
          "I can keep the old config or replace it. Do you want to keep it, or would you like to replace it?")


def sample(s, pane, label):
    st = s.status(pane)
    m = marker(s.cwd) or {}
    st = (st[0], st[1], m.get("status"), m.get("waitingFor"))
    ex = s.explain(pane)
    rule = ex.get("matched_rule") or ex.get("winning_rule") or {k: ex[k] for k in list(ex)[:6]}
    print(f"--- {label}: status={st}  explain={json.dumps(rule)[:400]}", flush=True)
    text = s.screen(pane)
    with open(os.path.join(OUT, label + ".txt"), "w") as f:
        f.write(text)
    return st[0]


def main():
    os.makedirs(OUT, exist_ok=True)
    with Session("typedblock") as s:
        pane = s.pane("claude")
        time.sleep(1)
        s.boot_claude(pane)
        sample(s, pane, "00-booted")
        s.send(pane, PROMPT)
        time.sleep(0.5)
        s.send(pane, "\r")
        time.sleep(3)
        s.wait_status(pane, "idle", 90)
        s.wait_status(pane, "done", 20)
        time.sleep(2)
        sample(s, pane, "01-answered-empty-composer")
        for i, word in enumerate(["p", "push", "push the branch"]):
            s.send(pane, word[len(["", "p", "push"][i]):])
            time.sleep(1.2)
            sample(s, pane, f"02-typed-{i}")
        s.send(pane, " and then " + "open a pull request against main with a long description " * 3)
        time.sleep(1.5)
        sample(s, pane, "03-typed-wrapped")
        s.send(pane, "\x7f" * 400)
        time.sleep(1.5)
        sample(s, pane, "04-erased")
        for label, keys in [("05-slash", "/"), ("06-at", "\x7f@"), ("07-bang", "\x7f!"), ("08-question", "\x7f?")]:
            s.send(pane, keys)
            time.sleep(1.5)
            sample(s, pane, label)
        s.send(pane, "\x7f\x1b")
        time.sleep(1)
        s.send(pane, "fix it")
        time.sleep(1)
        for t in range(10):
            sample(s, pane, f"09-typed-idle-{t}")
            time.sleep(3)
        print(json.dumps(s.explain(pane), indent=1)[:3000])


if __name__ == "__main__":
    main()
