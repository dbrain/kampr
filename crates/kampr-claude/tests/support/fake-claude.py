#!/usr/bin/env python3
"""A stand-in for `claude -p --input-format stream-json`, for tests that must never spend a turn.

It speaks the lines the real binary speaks (#644-#646), from a script the test writes beside it as
script.json, and it records what reached it, so a test can read what the source really sent:

    argv.log, env.json   how it was started
    stdin.log            every line the source wrote, in order
    answers.log          "ok" or "invalid: ..." for every control_response the source gave, judged
                         by the binary's own schema for a can_use_tool answer (2.1.326)

script.json: {"initialize": <body>, "usage": <body>, "turn": [<line> | {"sleep": s} | {"wait": "answer"}],
              "linger": s}
A turn is played when a user line arrives; an interrupt while it waits for an answer ends it with
an error result, as the binary does. With "linger", a hang-up takes that long to exit and then
logs ["exited"] to argv.log, so a test can see what started before it had gone.
"""
import json, os, signal, sys, time

here = os.path.dirname(os.path.abspath(__file__))
script = json.load(open(os.path.join(here, "script.json")))


def log(name, text):
    with open(os.path.join(here, name), "a") as f:
        f.write(text + "\n")


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def respond(request_id, body):
    emit({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": body}})


def invalid(body):
    behavior = body.get("behavior")
    if behavior == "allow":
        if "updatedInput" in body and not isinstance(body["updatedInput"], dict):
            return "updatedInput must be a record, got %r" % (body["updatedInput"],)
        if "updatedPermissions" in body and not isinstance(body["updatedPermissions"], list):
            return "updatedPermissions must be an array"
        return None
    if behavior == "deny":
        return None if isinstance(body.get("message"), str) else "deny needs a message"
    if "action" in body:
        return None if body["action"] in ("accept", "decline", "cancel") else "bad action"
    return "no such behavior %r" % (behavior,)


def session_id():
    args = sys.argv[1:]
    for flag in ("--session-id", "-r"):
        if flag in args:
            return args[args.index(flag) + 1]
    return "unnamed"


def write_transcript(prompt):
    root = os.environ.get("CLAUDE_CONFIG_DIR")
    if not root:
        return
    project = os.path.join(root, "projects", "-fake")
    os.makedirs(project, exist_ok=True)
    with open(os.path.join(project, session_id() + ".jsonl"), "a") as f:
        f.write(json.dumps({"type": "user", "uuid": "u1", "cwd": os.getcwd(), "timestamp": "2025-01-01T00:00:00Z", "message": {"content": prompt}}) + "\n")
        f.write(json.dumps({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "timestamp": "2025-01-01T00:00:01Z", "message": {"content": [{"type": "text", "text": "fake answer"}]}}) + "\n")


log("argv.log", json.dumps(sys.argv[1:]))
if "linger" in script:
    def hung_up(*_):
        time.sleep(script["linger"])
        log("argv.log", json.dumps(["exited"]))
        os._exit(0)
    signal.signal(signal.SIGHUP, hung_up)
with open(os.path.join(here, "env.json"), "w") as f:
    json.dump(dict(os.environ), f)

pending = []
suspended = False


def play():
    global suspended
    suspended = False
    while pending:
        step = pending.pop(0)
        if "sleep" in step:
            time.sleep(step["sleep"])
        elif step.get("wait") == "answer":
            suspended = True
            return
        else:
            emit(step)


for raw in sys.stdin:
    raw = raw.rstrip("\n")
    log("stdin.log", raw)
    line = json.loads(raw)
    kind = line.get("type")
    if kind == "control_request":
        sub = line["request"]["subtype"]
        if sub == "initialize":
            respond(line["request_id"], script["initialize"])
        elif sub == "get_usage":
            respond(line["request_id"], script.get("usage", {}))
        elif sub == "interrupt":
            respond(line["request_id"], {"still_queued": []})
            if suspended:
                pending.clear()
                suspended = False
                emit({"type": "result", "subtype": "error_during_execution", "is_error": True, "total_cost_usd": 0.0})
        else:
            respond(line["request_id"], {})
    elif kind == "user":
        content = line["message"]["content"]
        write_transcript(content if isinstance(content, str) else json.dumps(content))
        pending[:] = list(script.get("turn", []))
        play()
    elif kind == "control_response":
        body = line["response"].get("response", {})
        problem = invalid(body)
        log("answers.log", "ok" if problem is None else "invalid: " + problem)
        play()
