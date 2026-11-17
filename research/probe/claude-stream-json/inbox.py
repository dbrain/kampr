#!/usr/bin/env python3
"""Write lines to an interactive Claude's cross-session inbox and print whatever comes back.

    inbox.py <pid> [--no-auth] <json line>...

The socket and its key are read from ~/.claude/sessions/<pid>.json and <pid>.<hash>.key. Only ever
point this at a throwaway Claude: a line of type "user" lands in that session as a prompt.
"""
import glob, json, os, socket, sys

pid, args = sys.argv[1], sys.argv[2:]
auth = "--no-auth" not in args
lines = [a for a in args if a != "--no-auth"]
root = os.path.expanduser("~/.claude/sessions")
session = json.load(open(f"{root}/{pid}.json"))
key = json.load(open(glob.glob(f"{root}/{pid}.*.key")[0]))
print("key fields:", sorted(key))
conn = socket.socket(socket.AF_UNIX)
conn.connect(session["messagingSocketPath"])
conn.settimeout(2)
if auth:
    conn.sendall((json.dumps({"type": "auth", "token": key["peerToken"]}) + "\n").encode())
for line in lines:
    conn.sendall((line + "\n").encode())
got = b""
try:
    while chunk := conn.recv(65536):
        got += chunk
except socket.timeout:
    pass
print("reply:", got)
