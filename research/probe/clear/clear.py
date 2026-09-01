import sys, os, json, time, glob, signal, re
sys.path.insert(0, "research/probe")
from ptyclient import spawn, drain

WORK = "/tmp/kampr-clear-probe"
os.makedirs(WORK, exist_ok=True)
SLUG = os.path.expanduser("~/.claude/projects/" + WORK.replace("/", "-"))
SESS = os.path.expanduser("~/.claude/sessions")

def markers():
    out = {}
    for f in glob.glob(SESS + "/*.json"):
        try:
            d = json.load(open(f))
        except Exception:
            continue
        if d.get("cwd") == WORK:
            out[os.path.basename(f)] = (d.get("sessionId"), d.get("status"))
    return out

def files():
    return {os.path.basename(f)[:8]: os.path.getsize(f) for f in glob.glob(SLUG + "/*.jsonl")}

def show(label):
    print(f"{label:22} markers={markers()} files={files()}")

os.chdir(WORK)
pid, fd = spawn(["claude"], 100, 40)
out = drain(fd, 15)
tail = re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]", "", out.decode("utf8","replace")).splitlines()
print("screen:", [l.strip() for l in tail if l.strip()][-4:])
if any("trust" in l.lower() for l in tail):
    os.write(fd, b"1\r"); drain(fd, 10); print("answered the trust prompt")
show("after start")

os.write(fd, b"reply with exactly the word banana and nothing else\r")
drain(fd, 45)
show("after a prompt")

os.write(fd, b"/clear\r")
drain(fd, 6)
show("right after clear")
time.sleep(3)
show("3s after clear")

os.write(fd, b"reply with exactly the word cherry and nothing else\r")
drain(fd, 45)
show("after 2nd prompt")
time.sleep(2)
show("settled")
for f in sorted(glob.glob(SLUG + "/*.jsonl")):
    rows = [json.loads(l) for l in open(f, errors="replace") if l.strip().startswith("{")]
    ts = [r.get("timestamp") for r in rows if r.get("timestamp")]
    print(os.path.basename(f)[:8], "rows", len(rows), ts[:1], ts[-1:] )
os.kill(pid, signal.SIGKILL)
