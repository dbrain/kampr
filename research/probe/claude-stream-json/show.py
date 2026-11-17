#!/usr/bin/env python3
"""Print a drive.py log one line per record: time, direction, kind, and the fields worth reading.

    show.py <log.jsonl> [--deltas] [--full <kind-substring>]
"""
import json, sys

path, flags = sys.argv[1], sys.argv[2:]
deltas = "--deltas" in flags
full = flags[flags.index("--full") + 1] if "--full" in flags else None
DROP = {"t", "dir", "type", "subtype", "uuid", "session_id", "tools", "slash_commands", "skills", "agents",
        "plugins", "mcp_servers", "terminal_slash_commands", "memory_paths", "capabilities", "usage",
        "modelUsage", "model_usage_since_last_result"}
for line in open(path):
    o = json.loads(line)
    kind = o.get("type", "?")
    if o.get("subtype"):
        kind += "/" + o["subtype"]
    if kind in ("control_request", "control_response"):
        inner = o.get("request") or o.get("response") or {}
        kind += ":" + str(inner.get("subtype"))
    if kind == "stream_event":
        e = o["event"]
        kind += ":" + e["type"]
        if not deltas and e["type"] != "content_block_start":
            continue
        body = e.get("delta") or e.get("content_block") or {}
    elif kind in ("assistant", "user") and isinstance(o.get("message"), dict):
        body = {"content": o["message"].get("content"), "parent_tool_use_id": o.get("parent_tool_use_id"),
                "error": o.get("error"), "isApiErrorMessage": o.get("isApiErrorMessage")}
        body = {k: v for k, v in body.items() if v is not None}
    else:
        body = {k: v for k, v in o.items() if k not in DROP}
    text = json.dumps(body)
    if not (full and full in kind):
        text = text[:300]
    print(f"{o['t']:8.3f} {o['dir']} {kind:44} {text}")
