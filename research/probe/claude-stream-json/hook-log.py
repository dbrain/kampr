#!/usr/bin/env python3
"""A hook that appends its event, with the arrival time, to the file named in argv[1]."""
import json, sys, time

event = json.load(sys.stdin)
with open(sys.argv[1], "a") as log:
    log.write(json.dumps({"t": round(time.time(), 3), **event}) + "\n")
