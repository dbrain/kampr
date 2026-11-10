import json, os, sys, time
d = json.loads(sys.stdin.read())
log = os.path.join(os.path.dirname(os.path.abspath(__file__)), "run", "events.jsonl")
with open(log, "a") as f:
    f.write(json.dumps({"t": time.time(), "event": d["hook_event_name"], "tool_input": d.get("tool_input"), "suggestions": d.get("permission_suggestions")}) + "\n")
if d["hook_event_name"] == "PermissionRequest":
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": "allow", "updatedPermissions": d.get("permission_suggestions") or []}}}))
