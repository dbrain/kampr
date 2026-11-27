#!/usr/bin/env python3
import json, os, queue, subprocess, sys, threading, time

CODEX = "/home/dbrain/.local/bin/codex"
cdir = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "run6")
os.makedirs(os.path.join(cdir, "codex-home"), exist_ok=True)
mdir = os.path.join(os.path.dirname(cdir), "run", "mock")
os.makedirs(mdir, exist_ok=True)
open(os.path.join(mdir, "mode"), "w").write("ok")

proc = subprocess.Popen(
    [CODEX, "app-server", "--stdio",
     "-c", 'model_provider="mock"',
     '-c', 'model="mock-model"',
     '-c', 'model_providers.mock.name="mock"',
     '-c', 'model_providers.mock.base_url="http://127.0.0.1:8765/v1"',
     '-c', 'model_providers.mock.wire_api="responses"',
     '-c', 'model_providers.mock.env_key="MOCK_KEY"',
     ],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    env={**os.environ, "MOCK_KEY": "mock-key", "CODEX_HOME": os.path.join(cdir, "codex-home")},
    text=True, bufsize=1,
)

T0 = time.time()
out = open(os.path.join(cdir, "wire.log"), "w")
q = queue.Queue()

def reader():
    for line in proc.stdout:
        line = line.rstrip("\n")
        if line:
            out.write(f"<< {time.time()-T0:7.2f} {line}\n"); out.flush()
            try:
                q.put((time.time()-T0, json.loads(line)))
            except Exception:
                pass
threading.Thread(target=reader, daemon=True).start()

def drain_err():
    with open(os.path.join(cdir, "stderr.log"), "w") as f:
        for line in proc.stderr:
            f.write(line)
threading.Thread(target=drain_err, daemon=True).start()

next_id = [0]

def send(obj, tag=""):
    line = json.dumps(obj)
    proc.stdin.write(line + "\n"); proc.stdin.flush()
    out.write(f">> {time.time()-T0:7.2f} [{tag}] {line}\n"); out.flush()

def req(method, params, tag=""):
    next_id[0] += 1
    i = next_id[0]
    send({"jsonrpc": "2.0", "id": i, "method": method, "params": params}, tag)
    return i

def notify(method, tag=""):
    send({"jsonrpc": "2.0", "method": method, "params": None}, tag)

def wait_id(i, timeout=30, nudges=True):
    end = time.time() + timeout
    last_nudge = 0.0
    while time.time() < end:
        try:
            t, m = q.get(timeout=0.5)
        except queue.Empty:
            if nudges and time.time() - last_nudge > 2.0:
                req("account/read", {}, "nudge")
                last_nudge = time.time()
            continue
        if m.get("id") == i:
            return t, m
    return None, None

def collect(tag, seconds, stop=None):
    end = time.time() + seconds
    seen = []
    while time.time() < end:
        try:
            t, m = q.get(timeout=0.5)
        except queue.Empty:
            req("account/read", {}, "nudge")
            continue
        out.write(f"== {tag} t={t:7.2f} {json.dumps(m)[:400]}\n"); out.flush()
        seen.append((t, m))
        if stop and stop(m):
            break
    return seen

def set_mode(s):
    with open(os.path.join(mdir, "mode"), "w") as f:
        f.write(s)

def phase(tag, fn):
    print(f"--- {tag} ---", flush=True)
    fn()

def main():
    i = req("initialize", {"clientInfo": {"name": "kampr-probe", "version": "0.0.1"}, "capabilities": {}}, "A")
    t, m = wait_id(i)
    print(f"== A:initialize t={t} {json.dumps(m)[:300]}", flush=True)
    notify("initialized", "A")

    ids = {
        "models": req("model/list", {}, "A"),
        "account": req("account/read", {}, "A"),
        "config": req("config/read", {}, "A"),
        "usage": req("account/usage/read", {}, "A"),
    }
    req("account/read", {}, "A-flush")
    for name, i in ids.items():
        t, m = wait_id(i)
        body = json.dumps(m)
        print(f"== A:{name} t={t} len={len(body)} {body[:200]}", flush=True)

    def b2():
        i = req("thread/start", {"cwd": "/tmp", "approvalPolicy": "never", "sandboxPolicy": {"mode": "read-only"}, "model": "mock-model"}, "B")
        req("account/read", {}, "B-flush")
        t, m = wait_id(i)
        tid = m["result"]["thread"]["id"]
        print(f"== B:thread/start t={t} tid={tid}", flush=True)
        end = time.time() + 5
        while time.time() < end:
            try:
                t2, m2 = q.get(timeout=0.5)
            except queue.Empty:
                continue
            if m2.get("method") == "thread/started":
                print(f"== B:thread/started notif t={t2} {json.dumps(m2)[:200]}", flush=True)
                break
        return tid
    tid = b2()

    def c():
        set_mode("slow Say hello there friend")
        i = req("turn/start", {"threadId": tid, "input": [{"type": "text", "text": "Say hello"}]}, "C")
        req("account/read", {}, "C-flush")
        t, m = wait_id(i)
        print(f"== C:turn/start t={t} {json.dumps(m)[:400]}", flush=True)
        seen = collect("C", 20, stop=lambda m: m.get("method") == "turn/completed")
        print(f"== C:collected {len(seen)} msgs; last={json.dumps(seen[-1])[:300] if seen else None}", flush=True)
    phase("C:full turn (slow mock)", c)

    def d():
        i1 = req("thread/turns/list", {"threadId": tid, "limit": 10}, "D")
        i2 = req("thread/items/list", {"threadId": tid, "limit": 20}, "D")
        req("account/read", {}, "D-flush")
        t, m = wait_id(i1)
        print(f"== D:turns/list t={t} {json.dumps(m)[:400]}", flush=True)
        t, m = wait_id(i2)
        print(f"== D:items/list t={t} {json.dumps(m)[:600]}", flush=True)
    phase("D:lists", d)

    def e():
        set_mode("slow one two three four five six seven eight")
        i = req("turn/start", {"threadId": tid, "input": [{"type": "text", "text": "Slow"}]}, "E")
        req("account/read", {}, "E-flush")
        t, m = wait_id(i)
        turn_id = m["result"]["turn"]["id"] if m and m.get("result") else None
        print(f"== E:turn/start t={t} turnId={turn_id}", flush=True)
        time.sleep(3)
        j = req("turn/interrupt", {"threadId": tid, "turnId": turn_id}, "E")
        req("account/read", {}, "E-flush")
        t, m = wait_id(j)
        print(f"== E:turn/interrupt resp t={t} {json.dumps(m)[:200]}", flush=True)
        seen = collect("E", 15, stop=lambda m: m.get("method") == "turn/completed")
        print(f"== E:collected {len(seen)}; last={json.dumps(seen[-1])[:300] if seen else None}", flush=True)
    phase("E:interrupt", e)

    def f():
        set_mode("slow alpha beta gamma delta")
        i = req("turn/start", {"threadId": tid, "input": [{"type": "text", "text": "Steer me"}]}, "F")
        req("account/read", {}, "F-flush")
        t, m = wait_id(i)
        turn_id = m["result"]["turn"]["id"] if m and m.get("result") else None
        print(f"== F:turn/start t={t} turnId={turn_id}", flush=True)
        time.sleep(2)
        j = req("turn/steer", {"threadId": tid, "expectedTurnId": turn_id, "input": [{"type": "text", "text": "Now say bye"}]}, "F")
        req("account/read", {}, "F-flush")
        t, m = wait_id(j)
        print(f"== F:turn/steer resp t={t} {json.dumps(m)[:200]}", flush=True)
        seen = collect("F", 15, stop=lambda m: m.get("method") == "turn/completed")
        print(f"== F:collected {len(seen)}; last={json.dumps(seen[-1])[:300] if seen else None}", flush=True)
    phase("F:steer", f)

    def g():
        set_mode("error 429 rate limited by mock")
        i = req("turn/start", {"threadId": tid, "input": [{"type": "text", "text": "Fail me"}]}, "G")
        req("account/read", {}, "G-flush")
        t, m = wait_id(i)
        print(f"== G:turn/start(429) t={t} {json.dumps(m)[:300]}", flush=True)
        seen = collect("G", 10, stop=lambda m: m.get("method") == "turn/completed")
        print(f"== G:collected {len(seen)}; last={json.dumps(seen[-1])[:300] if seen else None}", flush=True)
    phase("G:provider error 429", g)

    def h():
        i = req("no/such/method", {}, "H")
        req("account/read", {}, "H-flush")
        t, m = wait_id(i)
        print(f"== H:unknown method t={t} {json.dumps(m)[:300]}", flush=True)
        i = req("thread/read", {"threadId": "does-not-exist"}, "H")
        j = req("turn/start", {"threadId": "does-not-exist", "input": [{"type": "text", "text": "x"}]}, "H")
        req("account/read", {}, "H-flush")
        t, m = wait_id(i)
        print(f"== H:thread/read(bad) t={t} {json.dumps(m)[:200]}", flush=True)
        t, m = wait_id(j)
        print(f"== H:turn/start(bad) t={t} {json.dumps(m)[:200]}", flush=True)
    phase("H:error surface", h)

    proc.stdin.close()
    time.sleep(1)
    proc.kill()
    print("DONE", flush=True)

main()