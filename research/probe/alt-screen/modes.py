import sys, os, re, time, signal
sys.path.insert(0, "research/probe")
from ptyclient import spawn, drain

MODES = {
    "1049": "alternate screen",
    "47": "alternate screen (old)",
    "1047": "alternate screen (1047)",
    "1000": "button mouse reporting",
    "1002": "drag mouse reporting",
    "1003": "any-motion mouse reporting",
    "1006": "SGR mouse encoding",
    "1015": "urxvt mouse encoding",
    "1004": "focus events",
    "2004": "bracketed paste",
    "1007": "alternate scroll",
    "25": "cursor visible",
}

pid, fd = spawn(["claude"], 100, 40, env={"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"})
out = drain(fd, 12)
os.kill(pid, signal.SIGKILL)
text = out.decode("utf-8", "replace")
set_ = re.findall(r"\x1b\[\?([0-9;]+)h", text)
reset = re.findall(r"\x1b\[\?([0-9;]+)l", text)
def flat(seqs):
    out = []
    for s in seqs:
        out.extend(s.split(";"))
    return out
print("bytes:", len(out))
print("SET  :", [f"{m} ({MODES.get(m,'?')})" for m in dict.fromkeys(flat(set_))])
print("RESET:", [f"{m} ({MODES.get(m,'?')})" for m in dict.fromkeys(flat(reset))])
