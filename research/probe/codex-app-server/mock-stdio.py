#!/usr/bin/env python3
"""A mock codex app-server: the probe's measured wire behaviour in a script.

Speaks newline-delimited JSON-RPC 2.0 on stdio and reproduces the real server's
transport quirk (probe row 3): a response — and the notifications that followed it —
is only flushed to stdout when the server next reads a line from stdin. The one
exception is `initialize`, which the real server answers immediately.

Persistence: $CODEX_HOME/threads.json — thread/start appends, thread/list and
thread/resume read it back, so a thread survives a process restart, as on the real
server (probe rows 13-16).

Modes (env KAMPR_MOCK_MODE):
  ok        (default) a turn streams "Hello from the mock codex." word by word
  slow      the same, 500 ms between words
  error429  the turn fails with the measured 429 shape (probe row 26)

Env:
  CODEX_HOME  where threads.json lives (default: the current directory)
"""
import json
import os
import sys
import threading
import time
import uuid

HOME = os.environ.get("CODEX_HOME", ".")
THREADS = os.path.join(HOME, "threads.json")
MODE = os.environ.get("KAMPR_MOCK_MODE", "ok")
STEP = 0.5 if MODE == "slow" else 0.1

REPLY_TEXT = "Hello from the mock codex."
STEER_TEXT = "Bye from the mock codex."

lock = threading.Lock()
pending = []  # lines waiting to be flushed, in order
active = None  # (thread_id, turn_id) of the running turn
cancel = None  # threading.Event for the running turn


def now_ms():
    return int(time.time() * 1000)


def load_threads():
    try:
        with open(THREADS) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_threads(threads):
    os.makedirs(HOME, exist_ok=True)
    tmp = THREADS + ".tmp"
    with open(tmp, "w") as f:
        json.dump(threads, f)
    os.replace(tmp, THREADS)


def emit(obj):
    """Queue one line for the next flush."""
    with lock:
        pending.append(json.dumps(obj))


def flush():
    """The one-behind rule: reading a new line releases everything queued before it."""
    with lock:
        out = "\n".join(pending)
        del pending[:]
    if out:
        sys.stdout.write(out + "\n")
        sys.stdout.flush()


def thread_obj(tid, t):
    return {
        "id": tid,
        "environments": [
            {
                "environmentId": "local",
                "cwd": t["cwd"],
                "runtimeWorkspaceRoots": [t["cwd"]],
            }
        ],
        "sessionId": tid,
        "forkedFromId": None,
        "attachedToThreadId": None,
        "parentThreadId": None,
        "preview": t.get("preview", ""),
        "ephemeral": False,
        "section": None,
        "projectId": None,
        "historyMode": "paginated",
        "modelProvider": "mock",
        "model": t.get("model"),
        "reasoningEffort": None,
        "createdAt": t["created"],
        "updatedAt": now_ms() // 1000,
        "recencyAt": now_ms() // 1000,
        "status": {"type": "idle"},
        "path": os.path.join(HOME, "sessions", tid + ".jsonl"),
        "cwd": t["cwd"],
        "cliVersion": "0.0.0-mock",
        "originator": "kampr-codex-mock",
        "source": "mock",
        "canAcceptDirectInput": True,
        "threadSource": None,
        "agentNickname": None,
        "agentRole": None,
        "gitInfo": None,
        "name": None,
        "daybreakEnabled": None,
        "turns": [],
    }


def user_item(text, started):
    return {
        "type": "userMessage",
        "id": str(uuid.uuid4()),
        "clientId": None,
        "content": [{"type": "text", "text": text, "text_elements": []}],
        "startedAtMs": started,
        "completedAtMs": started,
    }


def agent_item(text, started, completed):
    return {
        "type": "agentMessage",
        "moderation": None,
        "id": "msg_mock",
        "text": text,
        "phase": None,
        "memoryCitation": None,
        "delivery": None,
        "questions": None,
        "startedAtMs": started,
        "completedAtMs": completed,
    }


def text_of(input_list):
    return "".join(
        part.get("text", "") for part in (input_list or []) if part.get("type") == "text"
    )


def finish_turn(thread_id, turn_id, status, items, error, t0):
    emit(
        {
            "method": "thread/status/changed",
            "params": {"threadId": thread_id, "status": {"type": "idle"}},
            "emittedAtMs": now_ms(),
        }
    )
    emit(
        {
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": {
                    "id": turn_id,
                    "rootTurnId": turn_id,
                    "items": items,
                    "itemsView": "summary" if items else "notLoaded",
                    "status": status,
                    "error": error,
                    "startedAt": t0 // 1000,
                    "completedAt": now_ms() // 1000,
                    "durationMs": now_ms() - t0,
                },
            },
            "emittedAtMs": now_ms(),
        }
    )


def run_turn(thread_id, turn_id, user_text, reply_text):
    """The notification stream of one turn, queued into `pending` over time."""
    global active, cancel
    cancel = threading.Event()

    def work():
        t0 = now_ms()
        emit(
            {
                "method": "thread/status/changed",
                "params": {
                    "threadId": thread_id,
                    "status": {"type": "active", "activeFlags": []},
                },
                "emittedAtMs": now_ms(),
            }
        )
        emit(
            {
                "method": "turn/started",
                "params": {
                    "threadId": thread_id,
                    "turn": {
                        "id": turn_id,
                        "rootTurnId": turn_id,
                        "items": [],
                        "itemsView": "notLoaded",
                        "status": "inProgress",
                        "error": None,
                        "startedAt": now_ms() // 1000,
                        "completedAt": None,
                        "durationMs": None,
                    },
                },
                "emittedAtMs": now_ms(),
            }
        )
        if cancel.wait(STEP / 2):
            return

        if MODE == "error429":
            emit(
                {
                    "method": "thread/status/changed",
                    "params": {"threadId": thread_id, "status": {"type": "systemError"}},
                    "emittedAtMs": now_ms(),
                }
            )
            error = {
                "message": "exceeded retry limit, last status: 429 Too Many Requests",
                "codexErrorInfo": {
                    "responseTooManyFailedAttempts": {"httpStatusCode": 429}
                },
                "additionalDetails": None,
                "misalignment": None,
            }
            emit(
                {
                    "method": "error",
                    "params": {
                        "error": error,
                        "willRetry": False,
                        "threadId": thread_id,
                        "turnId": turn_id,
                    },
                    "emittedAtMs": now_ms(),
                }
            )
            finish_turn(thread_id, turn_id, "failed", [], error, t0)
            return

        u = user_item(user_text, now_ms())
        emit(
            {
                "method": "item/started",
                "params": {
                    "item": u,
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "startedAtMs": u["startedAtMs"],
                },
                "emittedAtMs": now_ms(),
            }
        )
        emit(
            {
                "method": "item/completed",
                "params": {
                    "item": u,
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "contentTokenCount": None,
                    "completedAtMs": u["startedAtMs"],
                },
                "emittedAtMs": now_ms(),
            }
        )
        if cancel.wait(STEP / 2):
            return

        a = agent_item("", now_ms(), None)
        emit(
            {
                "method": "item/started",
                "params": {
                    "item": a,
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "startedAtMs": a["startedAtMs"],
                },
                "emittedAtMs": now_ms(),
            }
        )
        for word in reply_text.split(" "):
            if cancel.wait(STEP):
                return  # interrupted; the main loop sends the interrupted completion
            emit(
                {
                    "method": "item/agentMessage/delta",
                    "params": {
                        "threadId": thread_id,
                        "turnId": turn_id,
                        "itemId": a["id"],
                        "delta": word + " ",
                    },
                    "emittedAtMs": now_ms(),
                }
            )
        a["text"] = reply_text
        a["completedAtMs"] = now_ms()
        emit(
            {
                "method": "item/completed",
                "params": {
                    "item": a,
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "contentTokenCount": None,
                    "completedAtMs": a["completedAtMs"],
                },
                "emittedAtMs": now_ms(),
            }
        )
        finish_turn(thread_id, turn_id, "completed", [a], None, t0)
        with lock:
            threads = load_threads()
            t = threads.setdefault(
                thread_id, {"cwd": "/tmp", "created": now_ms() // 1000, "model": None}
            )
            t["turns"].append(
                {
                    "id": turn_id,
                    "rootTurnId": turn_id,
                    "items": [a],
                    "status": "completed",
                    "error": None,
                    "startedAt": t0 // 1000,
                    "completedAt": a["completedAtMs"] // 1000,
                    "durationMs": a["completedAtMs"] - t0,
                }
            )
            if not t.get("preview"):
                t["preview"] = user_text
            save_threads(threads)

    threading.Thread(target=work, daemon=True).start()
    active = (thread_id, turn_id)


def handle_request(msg):
    global active, cancel
    method = msg.get("method")
    params = msg.get("params") or {}
    rid = msg.get("id")

    def respond(result):
        # queued, like the real server: the next line read is what flushes it
        emit({"jsonrpc": "2.0", "id": rid, "result": result})

    def fail(message):
        emit({"jsonrpc": "2.0", "id": rid, "error": {"code": -32600, "message": message}})

    if method == "initialize":
        # the one answer the real server writes without first being read
        sys.stdout.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": rid,
                    "result": {
                        "userAgent": "kampr-codex-mock/0.1",
                        "codexHome": HOME,
                        "platformFamily": "unix",
                        "platformOs": "linux",
                    },
                }
            )
            + "\n"
        )
        sys.stdout.flush()
        return

    if method == "initialized":
        return  # a notification: nothing to say

    if method == "thread/start":
        tid = str(uuid.uuid4())
        cwd = params.get("cwd") or "/tmp"
        threads = load_threads()
        threads[tid] = {
            "cwd": cwd,
            "created": now_ms() // 1000,
            "model": params.get("model"),
            "preview": "",
            "turns": [],
        }
        save_threads(threads)
        t = thread_obj(tid, threads[tid])
        respond(
            {
                "thread": t,
                "model": t["model"],
                "modelProvider": "mock",
                "serviceTier": None,
                "disabledPluginIds": [],
                "cwd": cwd,
                "runtimeWorkspaceRoots": [cwd],
                "instructionSources": [],
                "approvalPolicy": params.get("approvalPolicy"),
                "approvalsReviewer": "user",
                "sandbox": {"type": "readOnly", "networkAccess": False},
                "activePermissionProfile": {"id": ":read-only", "extends": None},
                "reasoningEffort": None,
                "multiAgentMode": "explicitRequestOnly",
            }
        )
        emit({"method": "thread/started", "params": {"thread": t}, "emittedAtMs": now_ms()})
        return

    if method == "turn/start":
        thread_id = params.get("threadId")
        if thread_id not in load_threads():
            fail("invalid thread id: no such thread")
            return
        turn_id = str(uuid.uuid4())
        respond(
            {
                "turn": {
                    "id": turn_id,
                    "rootTurnId": turn_id,
                    "items": [],
                    "itemsView": "notLoaded",
                    "status": "inProgress",
                    "error": None,
                    "startedAt": None,
                    "completedAt": None,
                    "durationMs": None,
                }
            }
        )
        run_turn(thread_id, turn_id, text_of(params.get("input")), REPLY_TEXT)
        return

    if method == "turn/interrupt":
        thread_id = params.get("threadId")
        turn_id = params.get("turnId")
        respond({})
        if cancel:
            cancel.set()
        cancel = None
        active = None
        finish_turn(thread_id, turn_id, "interrupted", [], None, now_ms())
        return

    if method == "turn/steer":
        thread_id = params.get("threadId")
        expected = params.get("expectedTurnId")
        if active != (thread_id, expected):
            fail("expectedTurnId does not match the active turn")
            return
        respond({"turnId": expected})
        if cancel:
            cancel.set()
        cancel = None
        run_turn(thread_id, expected, text_of(params.get("input")), STEER_TEXT)
        return

    if method == "thread/list":
        threads = load_threads()
        respond(
            {
                "data": [thread_obj(tid, t) for tid, t in threads.items()],
                "nextCursor": None,
                "backwardsCursor": None,
            }
        )
        return

    if method in ("thread/read", "thread/resume"):
        threads = load_threads()
        t = threads.get(params.get("threadId"))
        if t is None:
            fail("no such thread: %s" % params.get("threadId"))
            return
        respond({"thread": thread_obj(params["threadId"], t)})
        return

    if method == "thread/turns/list":
        t = load_threads().get(params.get("threadId"))
        if t is None:
            fail("no such thread: %s" % params.get("threadId"))
            return
        data = [
            {
                "id": turn["id"],
                "rootTurnId": turn["rootTurnId"],
                "items": turn.get("items", []),
                "itemsView": "summary",
                "status": turn["status"],
                "error": turn.get("error"),
                "startedAt": turn.get("startedAt"),
                "completedAt": turn.get("completedAt"),
                "durationMs": turn.get("durationMs"),
            }
            for turn in reversed(t.get("turns", []))
        ]
        respond({"data": data, "nextCursor": None, "backwardsCursor": None})
        return

    if method == "account/read":
        respond(
            {
                "account": None,
                "userId": None,
                "requiresOpenaiAuth": False,
                "workspaceRouting": None,
            }
        )
        return

    if method == "config/read":
        respond({"config": {"model": "mock-model", "model_provider": "mock"}})
        return

    fail("Invalid request: unknown variant `%s`" % method)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        flush()  # reading a new line releases everything queued before it
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if "id" not in msg:
            continue  # a notification (initialized): answered with silence
        handle_request(msg)
        # EOF: the pipe is closed, the thread's process is done


if __name__ == "__main__":
    main()