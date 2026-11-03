#!/usr/bin/env python3
# Compose Multiplatform injects one <style> into its shadow root, and the CSP admits it by hash. A
# CMP upgrade that changes a byte of it gets the stylesheet refused with nothing but a console line:
# 1.12 added the offscreen input's rules to it, and the stale hash left that input unstyled.
import base64
import glob
import hashlib
import re
import sys

dist, *policies = sys.argv[1:]

styles = set()
for wasm in glob.glob(f"{dist}/*.wasm"):
    data = open(wasm, "rb").read()
    for found in re.finditer(rb" :host \{", data):
        # A Kotlin/Wasm string literal is its LEB128 byte length followed by the bytes.
        width = 1
        while data[found.start() - width - 1] & 0x80:
            width += 1
        prefix = data[found.start() - width : found.start()]
        length = sum((b & 0x7F) << (7 * i) for i, b in enumerate(prefix))
        styles.add(data[found.start() : found.start() + length])
if len(styles) != 1:
    sys.exit(f"expected one Compose shadow-root stylesheet in {dist}/*.wasm, found {len(styles)}")

digest = "sha256-" + base64.b64encode(hashlib.sha256(styles.pop()).digest()).decode()
stale = [p for p in policies if f"style-src 'self' '{digest}'" not in open(p, encoding="utf-8").read()]
for path in stale:
    print(f"error: {path} does not admit Compose's stylesheet; its style-src needs '{digest}'", file=sys.stderr)
if stale:
    sys.exit(1)
print(f"wasm csp: Compose's stylesheet is {digest}, admitted by {len(policies)} policies")
