#!/usr/bin/env python3
"""How often herdr holds a `pane.send_text` made the node's way (#450) for its 100 ms (#445), at a
finger's 16 ms cadence and at 60 ms, into a Claude pane that is repainting and one that is not.
The carried scroll pump cannot absorb one (#572), so its rate is the pump's exposure.

Throwaway named session, torn down at the end (#97). Usage: wheel-stalls.py
"""
import os, sys, time
from importlib import import_module

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
wc = import_module("wheel-carry")
wb = wc.wb
UP, DOWN = wc.UP, wc.DOWN


def run(pane, label, n, gap, texts):
    before = len(wc.STALLS)
    took = []
    t0 = time.monotonic()
    for i in range(n):
        wc.until(t0 + i * gap / 1000)
        a = time.monotonic()
        wc.send(pane, texts(i))
        took.append((time.monotonic() - a) * 1000)
    s = len(wc.STALLS) - before
    took.sort()
    print(f"  {label:<40} n={n} gap={gap}ms stalls={s} ({100 * s / n:.1f}%)"
          f" p50={took[n // 2]:.2f}ms p99={took[int(n * .99)]:.1f}ms", flush=True)


def main():
    pane, frames, cwd = wb.transcript(300)
    try:
        wb.call("pane.send_text", {"pane_id": pane, "text": "\x1b[1;5H"})
        time.sleep(1)
        painting = lambda i: (UP if (i // 20) % 2 else DOWN) * 2
        for _ in range(2):
            run(pane, "at top, nothing repaints, 16ms", 400, 16, lambda i: UP)
            run(pane, "scrolling, Claude repaints, 16ms", 400, 16, painting)
            run(pane, "scrolling, Claude repaints, 60ms", 200, 60, painting)
            run(pane, "at top, nothing repaints, 60ms", 200, 60, lambda i: UP)
        frames.close()
    finally:
        wb.composer.stop()
        import shutil
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
