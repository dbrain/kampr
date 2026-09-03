"""Slices a KAMPR_CALL_LOG by the phases phases.py marked, and counts calls per method.

`pane.read` is broken out by source and format because that is what separates its three callers:
the width probe reads `recent` and `recent_unwrapped` as text, the scrollback pump reads `recent`
as ansi, and the pending strip reads `visible`.
"""
import sys, collections
calls = [l.rstrip("\n").split("\t") for l in open(sys.argv[1])]
marks = [l.rstrip("\n").split("\t") for l in open(sys.argv[2])]
phases = []
begin = None
for t, name in marks:
    if name.startswith("begin "):
        begin = (float(t), name[6:])
    elif name.startswith("end ") and begin:
        phases.append((begin[0], float(t), begin[1]))
        begin = None
for lo, hi, name in phases:
    win = [c for c in calls if lo <= float(c[0]) <= hi]
    n = collections.Counter()
    for c in win:
        method = c[1]
        key = method
        if method == "pane.read":
            key = f"pane.read[{c[3]}/{c[4]}]"
        n[key] += 1
    dur = hi - lo
    print(f"\n== {name}  ({dur:.0f}s, {len(win)} calls, {len(win)/dur:.2f}/s)")
    for k, v in n.most_common():
        print(f"   {v:5d}  {v/dur*60:7.1f}/min  {k}")
