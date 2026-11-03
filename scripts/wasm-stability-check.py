#!/usr/bin/env python3
# Kotlin/Wasm sets a class's Compose `$stable` inside its file's lazy property initialiser whenever
# that file has one, and the `$stable` getter another module calls does not run it. The caller
# then reads "unstable" on the first composition and "stable" on the next, reads a different
# number of slots, and traps (BrowserComposerFirstFrameTest; the reply box's null-pointer and its
# stalled typing). `@Immutable` or `@Stable` makes the stability static, so nothing reads the
# getter. Only the compiled program can say which classes are exposed, so this reads it.
import glob
import os
import re
import subprocess
import sys
import tempfile

wasm = sys.argv[1]
dis = os.environ.get("WASM_DIS") or max(
    glob.glob(os.path.expanduser("~/.gradle/binaryen/binaryen-version_*/bin/wasm-dis")),
    key=lambda p: int(re.search(r"version_(\d+)", p).group(1)),
    default=None,
)
if dis is None:
    sys.exit("no wasm-dis: build the web bundle once so Kotlin fetches binaryen, or set WASM_DIS")

with tempfile.NamedTemporaryFile(suffix=".wat") as wat:
    subprocess.run([dis, wasm, "-o", wat.name], check=True)
    lines = open(wat.name, encoding="utf-8").read().split("\n")

getters, lazy, func = set(), {}, None
for i, line in enumerate(lines):
    started = re.match(r"\s*\(func \$(\S+)", line)
    if started:
        func = started.group(1)
        getter = re.match(r"(dev\.kampr\..*\$stable)prop_getter$", func)
        if getter:
            getters.add(getter.group(1))
        continue
    stored = re.search(r"global\.set \$(dev\.kampr\.\S*\$stable)\s*$", line)
    if stored and func and "<init\\20properties" in func and "(i32.const 0)" not in lines[i + 1]:
        lazy[stored.group(1)] = func

exposed = sorted(getters & lazy.keys())
for name in exposed:
    print(f"error: {name} is read by another module but set in {lazy[name]}", file=sys.stderr)
if exposed:
    sys.exit("mark each class @Immutable or @Stable (scripts/wasm-stability-check.py says why)")
print(f"wasm stability: {len(getters)} cross-module $stable getters, none lazily set")
