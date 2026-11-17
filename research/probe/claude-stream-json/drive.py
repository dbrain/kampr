#!/usr/bin/env python3
"""Drive a headless Claude over stream-json and log every line with its arrival time.

    drive.py <out.jsonl> [--env K=V]... [--arg X]... -- <step>...

Steps run in order:
    say:<text>          send a user message
    ctl:<json>          send a control_request with that body, log the response; $TASK is the
                        task_id of the latest task_started
    wait:<type>[/<subtype>][:<secs>]   block until such a line arrives (default 90 s)
    sleep:<secs>
    eof                 close stdin

A can_use_tool request is answered from --allow (default), --deny, or --answer <json> where the
json is the whole `response` body. Each log line is {"t": secs since start, "dir": "<"|">", ...}.
"""
import json, os, subprocess, sys, threading, time, uuid

out_path, rest = sys.argv[1], sys.argv[2:]
split = rest.index("--")
opts, steps = rest[:split], rest[split + 1:]
env, extra, mode, answer = dict(os.environ), [], "allow", None
i = 0
while i < len(opts):
    o = opts[i]
    if o == "--env":
        k, v = opts[i + 1].split("=", 1); env[k] = v; i += 2
    elif o == "--unset":
        env.pop(opts[i + 1], None); i += 2
    elif o == "--arg":
        extra.append(opts[i + 1]); i += 2
    elif o == "--answer":
        answer = json.loads(opts[i + 1]); i += 2
    elif o in ("--allow", "--deny", "--hold"):
        mode = o[2:]; i += 1
    else:
        sys.exit(f"unknown option {o}")

cmd = [env.get("CLAUDE_BIN", "claude"), "-p", "--input-format", "stream-json", "--output-format",
       "stream-json", "--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio",
       *extra]
proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                        env=env, text=True, bufsize=1)
t0, log, seen, cond = time.time(), open(out_path, "w"), [], threading.Condition()


def record(direction, obj):
    log.write(json.dumps({"t": round(time.time() - t0, 3), "dir": direction, **obj}) + "\n")
    log.flush()


def send(obj):
    record(">", obj)
    proc.stdin.write(json.dumps(obj) + "\n")
    proc.stdin.flush()


def reader():
    for line in proc.stdout:
        try:
            obj = json.loads(line)
        except ValueError:
            obj = {"type": "raw", "line": line.rstrip()}
        record("<", obj)
        req = obj.get("request", {})
        if obj.get("type") == "control_request" and req.get("subtype") == "can_use_tool" and mode != "hold":
            body = answer or ({"behavior": "allow", "updatedInput": req.get("input", {})}
                              if mode == "allow" else {"behavior": "deny", "message": "probe said no"})
            send({"type": "control_response",
                  "response": {"subtype": "success", "request_id": obj["request_id"], "response": body}})
        with cond:
            seen.append(obj)
            cond.notify_all()
    with cond:
        seen.append({"type": "eof"})
        cond.notify_all()


def stderr_reader():
    for line in proc.stderr:
        record("<", {"type": "stderr", "line": line.rstrip()})


threading.Thread(target=reader, daemon=True).start()
threading.Thread(target=stderr_reader, daemon=True).start()
cursor = 0
for step in steps:
    kind, _, arg = step.partition(":")
    if kind == "say":
        send({"type": "user", "message": {"role": "user", "content": arg}, "parent_tool_use_id": None,
              "session_id": ""})
    elif kind == "ctl":
        with cond:
            task = next((o["task_id"] for o in reversed(seen) if o.get("subtype") == "task_started"), "")
        send({"type": "control_request", "request_id": uuid.uuid4().hex[:8],
              "request": json.loads(arg.replace("$TASK", task))})
    elif kind == "sleep":
        time.sleep(float(arg))
    elif kind == "eof":
        proc.stdin.close()
    elif kind == "wait":
        want, _, secs = arg.partition(":")
        typ, _, sub = want.partition("/")
        deadline = time.time() + float(secs or 90)
        with cond:
            while True:
                hit = next((n for n in range(cursor, len(seen))
                            if seen[n].get("type") in (typ, "eof")
                            and (not sub or seen[n].get("subtype") == sub
                                 or seen[n].get("request", {}).get("subtype") == sub
                                 or seen[n].get("type") == "eof")), None)
                if hit is not None:
                    cursor = hit + 1
                    break
                if not cond.wait(max(0, deadline - time.time())) and time.time() >= deadline:
                    record("!", {"type": "timeout", "waiting_for": want})
                    break
try:
    proc.stdin.close()
except Exception:
    pass
try:
    proc.wait(20)
except subprocess.TimeoutExpired:
    proc.kill()
time.sleep(0.3)
record("!", {"type": "exit", "code": proc.returncode})
