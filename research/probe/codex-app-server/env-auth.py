#!/usr/bin/env python3
"""Which environment variables change codex app-server's auth or endpoint, with no turn and no
network: each case boots `codex app-server --stdio` under `unshare -rn` with a throwaway HOME
and CODEX_HOME and one variable set, then reads its account, auth status, config and providers.

    env-auth.py <codex binary> <out dir> [NAME=value ...]   (no NAME: the built-in cases)
"""
import json, os, select, subprocess, sys, tempfile, time

FAKE = "sk-proj-kampr-probe-not-a-real-key"
CASES = [
    {},
    {"OPENAI_API_KEY": FAKE},
    {"CODEX_API_KEY": FAKE},
    {"CODEX_ACCESS_TOKEN": FAKE},
    {"OPENAI_API_KEY": FAKE, "CODEX_API_KEY": FAKE},
    {"OPENAI_BASE_URL": "http://127.0.0.1:9/probe-base"},
    {"OPENAI_ORGANIZATION": "org-kampr-probe", "OPENAI_PROJECT": "proj-kampr-probe"},
    {"AWS_BEARER_TOKEN_BEDROCK": FAKE},
    {"CODEX_OSS_BASE_URL": "http://127.0.0.1:9/probe-oss", "CODEX_OSS_PORT": "9"},
    {"CODEX_APP_SERVER_CHATGPT_BASE_URL": "http://127.0.0.1:9/probe-chatgpt"},
    {"AZURE_OPENAI_API_KEY": FAKE},
]
ASKS = [
    ("account/read", {}),
    ("getAuthStatus", {"includeToken": False, "refreshToken": False}),
    ("config/read", {}),
    ("modelProvider/list", {}),
]


def run(codex, extra):
    root = tempfile.mkdtemp(prefix="codex-env-")
    home, chome = os.path.join(root, "home"), os.path.join(root, "codex-home")
    os.makedirs(home), os.makedirs(chome)
    env = {"PATH": "/usr/bin:/bin", "HOME": home, "CODEX_HOME": chome, **extra}
    p = subprocess.Popen(["unshare", "-rn", codex, "app-server", "--stdio"], env=env,
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    lines = [{"jsonrpc": "2.0", "id": 1, "method": "initialize",
              "params": {"clientInfo": {"name": "kampr-probe", "version": "0"}, "capabilities": {}}},
             {"jsonrpc": "2.0", "method": "initialized"}]
    lines += [{"jsonrpc": "2.0", "id": 10 + i, "method": m, "params": a} for i, (m, a) in enumerate(ASKS)]
    for l in lines:
        p.stdin.write((json.dumps(l) + "\n").encode())
    p.stdin.flush()
    got, buf, nudge, end = {}, b"", 100, time.time() + 15
    while time.time() < end and not all(10 + i in got for i in range(len(ASKS))):
        r, _, _ = select.select([p.stdout], [], [], 0.25)
        if not r:
            nudge += 1
            p.stdin.write((json.dumps({"jsonrpc": "2.0", "id": nudge, "method": "account/read", "params": {}}) + "\n").encode())
            p.stdin.flush()
            continue
        chunk = os.read(p.stdout.fileno(), 65536)
        if not chunk:
            break
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if isinstance(m.get("id"), int) and 10 <= m["id"] < 100:
                got[m["id"]] = m.get("result", m.get("error"))
    p.stdin.close()
    p.wait(timeout=10)
    return {m: got.get(10 + i) for i, (m, _) in enumerate(ASKS)}


def main():
    codex, out = sys.argv[1], sys.argv[2]
    cases = [dict(a.split("=", 1) for a in sys.argv[3:])] if len(sys.argv) > 3 else CASES
    results = [{"env": c, **run(codex, c)} for c in cases]
    with open(out, "w") as f:
        json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
