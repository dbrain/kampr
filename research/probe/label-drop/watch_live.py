#!/usr/bin/env python3
"""Read-only watch of the operator's live claude panes. No writes of any kind."""
import json, os, sys, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
S = "/home/dbrain/.config/herdr/herdr.sock"
DUR = float(sys.argv[1]) if len(sys.argv) > 1 else 240.0
panes = [p["pane_id"] for p in rpc("pane.list", {}, sock_path=S)["result"]["panes"]]
rows = []
end = time.time() + DUR
lastex = 0.0
while time.time() < end:
    t = time.time()
    rec = {"t": t}
    for p in panes:
        e = {}
        try:
            g = rpc("pane.get", {"pane_id": p}, sock_path=S)["result"]["pane"]
            e["agent"], e["status"], e["title"] = g.get("agent"), g.get("agent_status"), g.get("terminal_title")
        except Exception as ex: e["err"] = str(ex)
        try:
            pi = rpc("pane.process_info", {"pane_id": p}, sock_path=S)["result"]["process_info"]
            e["fg"] = [x["name"] for x in pi.get("foreground_processes", [])]
            e["pgid"] = pi.get("foreground_process_group_id")
        except Exception as ex: e["fg"] = ["ERR"]
        rec[p] = e
    if t - lastex > 1.0:
        lastex = t
        for p in panes:
            try:
                ex = rpc("agent.explain", {"target": p}, sock_path=S)["result"]["explain"]
                osc = ""
                for r in ex.get("evaluated_rules", []):
                    if r.get("region") == "osc_title":
                        osc = r["evidence"].get("region_preview"); break
                mr = ex.get("matched_rule")
                rec[p]["ex"] = {"fb": ex.get("fallback_reason"), "osc": osc,
                                "rule": mr.get("id") if isinstance(mr, dict) else mr,
                                "state": ex.get("state")}
            except Exception as e:
                rec[p]["ex"] = {"err": str(e)}
    rows.append(rec)
    d = 0.15 - (time.time() - t)
    if d > 0: time.sleep(d)
json.dump({"panes": panes, "rows": rows}, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "live.json"), "w"))
print("done", len(rows))
