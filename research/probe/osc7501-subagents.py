#!/usr/bin/env python3
"""What Claude Code writes over OSC 7501 for work it runs beside its own turn: a subagent, a background
subagent, a subagent that asks for a permission, one interrupted, and /clear (docs/24).

    python3 -m venv /tmp/v && /tmp/v/bin/pip install pyte
    /tmp/v/bin/python research/probe/osc7501-subagents.py

One claude under a throwaway CLAUDE_CONFIG_DIR on a pty this script owns, with 7501 answered, talking to
claude-signals.py's mock Messages endpoint extended to script the parent's Agent/Task call and the
subagent's own requests. No real model is called. Prints each OSC 7501 report in stream order with
the phase it fell in.
"""

import importlib.util
import json
import os
import re
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("signals", os.path.join(HERE, "claude-signals.py"))
signals = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signals)

REPORT = re.compile(rb"\x1b\]7501;([^\x07\x1b]*)(?:\x07|\x1b\\)")


def first_user_text(body):
    users = [m for m in body.get("messages", []) if m.get("role") == "user"]
    return json_text(users[0]) if users else ""


def json_text(obj):
    import json
    return json.dumps(obj)


def agent_tool(body):
    names = [t.get("name") for t in body.get("tools", [])]
    return next((n for n in ("Agent", "Task") if n in names), None)


class Mock(signals.Mock):
    """Four prompts: SUBS (a subagent), BGGO (a background one), WPERM (a subagent that runs Bash and so
    asks), SLOWSUB (a subagent that talks for a long time). A request whose first user message holds
    WORKER is the subagent's own."""

    def do_POST(self):
        import json
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        text = json_text(signals.last_user(body))
        first = json_text(body.get("messages", [{}])[0])[-400:]
        done = signals.results_in(signals.last_user(body))
        if self.path.split("?")[0].endswith("count_tokens") or not body.get("stream"):
            return self.json({"input_tokens": 10, "id": "msg_p", "type": "message", "role": "assistant",
                              "model": body.get("model"), "content": [{"type": "text", "text": '{"title": "Probe"}'}],
                              "stop_reason": "end_turn", "stop_sequence": None,
                              "usage": {"input_tokens": 10, "output_tokens": 3}})
        tool = agent_tool(body)
        if "WORKER" in first:
            if "toolu_wperm" in done:
                return self.stream("Worker made its file.", None)
            if "WPERM" in first and not done:
                return self.stream("Worker needs a file. ", ("toolu_wperm", "Bash", {
                    "command": "touch worker-made-this.txt", "description": "Worker file"}))
            if "SLOWSUB" in first:
                return self.stream("Worker words. " * 150, None)
            return self.stream("Worker words, a few of them. " * 8, None)
        answered = {b.get("tool_use_id") for m in body.get("messages", []) if m.get("role") == "user"
                    and isinstance(m.get("content"), list) for b in m["content"] if b.get("type") == "tool_result"}
        for marker, background in (("SUBS", False), ("BGGO", True), ("WPERM", False), ("SLOWSUB", False)):
            if marker in text and "toolu_" + marker.lower() not in answered and tool:
                inp = {"description": "probe worker", "prompt": f"WORKER {marker} do it", "subagent_type": "general-purpose"}
                if background:
                    inp["run_in_background"] = True
                return self.stream("Starting a worker. ", ("toolu_" + marker.lower(), tool, inp))
        if done:
            return self.stream("PARENTDONE " + "Closing words. " * 4, None)
        if "compact" in json_text(body.get("system", "")).lower():
            return self.stream("<summary>Probe summary.</summary>", None)
        return self.stream("Short answer.", None)


class Run(signals.Run):
    def __init__(self, *a):
        super().__init__(*a)
        self.watch.append("PARENTDONE")

    def step(self, name, prompt, wait_for=None, wait=30, settle=3):
        self.phase(name)
        self.type(prompt)
        time.sleep(0.5)
        os.write(self.fd, b"\r")
        if wait_for:
            self.until(wait_for, wait)
        time.sleep(settle)

    def script(self):
        if not self.until("\u276f", 25):
            return
        time.sleep(2.5)
        self.step("subagent", "SUBS go", "PARENTDONE", 40)
        self.seen.pop("PARENTDONE", None)
        self.step("background", "BGGO go", None, settle=12)
        self.phase("worker-permission")
        self.type("WPERM go")
        time.sleep(0.5)
        os.write(self.fd, b"\r")
        if self.until("Do you want", 40):
            time.sleep(3)
            self.phase("worker-approve")
            os.write(self.fd, b"\r")
        time.sleep(8)
        self.step("slow-worker", "SLOWSUB go", None, settle=1)
        self.phase("escape")
        os.write(self.fd, b"\x1b")
        time.sleep(25)
        self.step("clear", "/clear", None, settle=4)
        self.phase("exit")
        self.type("/exit")
        time.sleep(0.6)
        os.write(self.fd, b"\r")
        time.sleep(4)


OSC = re.compile(rb"\x1b\](\d+);")


def main():
    """With `--token`, the run's claude also has an `OSC7577_TOKEN`, as a native session's would, and
    every OSC number it wrote is counted."""
    mock = signals.serve(Mock, requests=[])
    variant = json.loads(json.dumps(signals.VARIANTS["osc7501"]))
    if "--token" in sys.argv:
        variant["env"]["OSC7577_TOKEN"] = "probe-token-0123456789abcdef"
    run = Run("osc7501", variant, tempfile.mkdtemp(prefix="k75s-"), mock.server_address[1])
    try:
        run.script()
    finally:
        phases = sorted((t, name) for name, t in run.seen.items() if name.startswith("@"))
        rows = []
        whole = b"".join(c for _, c in run.log)
        codes = {}
        for m in OSC.finditer(whole):
            codes[m.group(1).decode()] = codes.get(m.group(1).decode(), 0) + 1
        for t, chunk in run.log:
            for m in REPORT.finditer(chunk):
                rows.append((run.rel(t), m.group(1).decode("latin1")))
        run.close()
        mock.shutdown()
    pi = 0
    for t, body in rows:
        while pi < len(phases) and phases[pi][0] <= t:
            print(f"--- {phases[pi][1][1:]} @ {phases[pi][0]}")
            pi += 1
        print(f"{t:8.3f} {body}")
    for t, name in phases[pi:]:
        print(f"--- {name[1:]} @ {t}")
    print("OSC numbers written:", dict(sorted(codes.items(), key=lambda kv: int(kv[0]))))


if __name__ == "__main__":
    sys.exit(main())
