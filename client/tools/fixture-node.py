#!/usr/bin/env python3
"""A Kampr node fixture: /ws speaks docs/04-wire-protocol.md, / serves the wasm bundle.

No kampr-node crate exists yet, so this replays the documented message set so the client
can be driven and screenshotted end to end. Stdlib only.
"""
import asyncio
import base64
import hashlib
import json
import mimetypes
import os
import sys
import time

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

# The node's policy: no external origins at all. wasm-unsafe-eval is required by Skiko.
CSP = (
    "default-src 'self'; "
    "script-src 'self' 'wasm-unsafe-eval'; "
    # Compose Multiplatform injects one :host stylesheet into its shadow root at runtime.
    "style-src 'self' 'sha256-+bHRyQ0Z1/Lb6dgSILtTESBRCIFl8jkBb/dPQA4Pdnw='; "
    "img-src 'self' data: blob:; "
    "font-src 'self'; "
    "connect-src 'self' ws: wss:; "
    "worker-src 'self' blob:; "
    "object-src 'none'; "
    "base-uri 'none'"
)
NODE = "01JKAMPRNODE0000000000000"
PEER1 = "01JKAMPRNODE0000000000001"
PEER2 = "01JKAMPRNODE0000000000002"

ROOT = sys.argv[1] if len(sys.argv) > 1 else "."
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8790


def iso(offset):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - offset))


def herd():
    return {
        "t": "herd",
        "nodes": [
            {"id": NODE, "name": "comingclean", "kind": "local", "online": True, "rtt_ms": 0.4,
             "herdr_version": "0.8.2"},
            {"id": PEER1, "name": "sungrow-pi", "kind": "peer", "online": True, "rtt_ms": 38.0,
             "herdr_version": "0.8.2"},
            {"id": PEER2, "name": "nas", "kind": "peer", "online": True, "rtt_ms": 6.0,
             "herdr_version": "0.8.2"},
        ],
        "panes": [
            {"id": f"{NODE}/w3:p2", "node_id": NODE, "workspace": "kampr", "tab": "1",
             "cwd": "~/dev/kampr", "label": None, "agent": "claude", "agent_status": "blocked",
             "cols": 89, "rows": 34, "scrollback_rows": 0, "has_conversation": True,
             "updated_at": iso(3), "unknown_future_field": 7},
            {"id": f"{NODE}/w4:p1", "node_id": NODE, "workspace": "kob", "tab": "1",
             "cwd": "~/dev/tinyfiddler/kob", "label": None, "agent": "codex",
             "agent_status": "working", "cols": 94, "rows": 30, "scrollback_rows": 0,
             "has_conversation": True, "updated_at": iso(120)},
            {"id": f"{NODE}/w3:p1", "node_id": NODE, "workspace": "kampr", "tab": "1",
             "cwd": "~/dev/kampr", "label": None, "agent": None, "agent_status": "idle",
             "cols": 94, "rows": 40, "scrollback_rows": 1553, "has_conversation": False,
             "updated_at": iso(2460)},
            {"id": f"{PEER1}/w1:p1", "node_id": PEER1, "workspace": "sungrow", "tab": "1",
             "cwd": "~/dev/sungrow", "label": None, "agent": "claude", "agent_status": "done",
             "cols": 120, "rows": 40, "scrollback_rows": 0, "has_conversation": True,
             "updated_at": iso(720)},
            {"id": f"{PEER2}/w1:p1", "node_id": PEER2, "workspace": "houseofdoge", "tab": "1",
             "cwd": "~/dev/houseofdoge", "label": None, "agent": None, "agent_status": "idle",
             "cols": 100, "rows": 30, "scrollback_rows": 400, "has_conversation": False,
             "updated_at": iso(10800)},
        ],
    }


STYLES = {
    "t": "styles", "from": 1,
    "styles": [
        {"fg": {"k": "r", "v": [88, 214, 141]}, "bg": {"k": "d"}},                 # 1 done
        {"fg": {"k": "r", "v": [84, 90, 104]}, "bg": {"k": "d"}},                  # 2 mute
        {"fg": {"k": "r", "v": [110, 168, 254]}, "bg": {"k": "d"}},                # 3 accent
        {"fg": {"k": "r", "v": [141, 147, 163]}, "bg": {"k": "d"}},                # 4 dim
        {"fg": {"k": "r", "v": [232, 234, 240]}, "bg": {"k": "d"}, "bold": True},  # 5 bold
        {"fg": {"k": "r", "v": [9, 10, 13]}, "bg": {"k": "r", "v": [232, 234, 240]}},  # 6 cursor
        {"fg": {"k": "r", "v": [255, 200, 87]}, "bg": {"k": "d"}, "underline": True},  # 7 link
    ],
}

TERMINAL = [
    [(1, "\u25cf"), (0, " Read bridge/server.ts (412 lines)")],
    [],
    [(1, "\u25cf"), (0, " The write path needs the device check in two places, not one.")],
    [(0, "  Adding both, then wiring the audit line.")],
    [],
    [(2, "\u256d\u2500 "), (3, "Edit"), (2, " \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u256e")],
    [(2, "\u2502"), (0, " bridge/server.ts"), (2, "                                                \u2502")],
    [(2, "\u2502 "), (2, "142"), (1, " +  if (!device.canWrite) {"), (2, "                                \u2502")],
    [(2, "\u2502 "), (2, "143"), (1, " +    return json(403, \"read-only\")"), (2, "                        \u2502")],
    [(2, "\u2502 "), (2, "144"), (1, " +  }"), (2, "                                                     \u2502")],
    [(2, "\u2502 "), (2, "145"), (0, "    audit(device, \"pane.send_text\", pane)"), (2, "                  \u2502")],
    [(2, "\u2570\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u256f")],
    [],
    [(0, "Do you want to make this edit?")],
    [],
    [(3, "\u276f 1."), (0, " Yes")],
    [(4, "  2."), (0, " Yes, and don't ask again")],
    [(4, "  3."), (0, " No, tell Claude what to do differently"), (6, " ")],
    [],
    [(2, "  see "), (7, "https://herdr.dev"), (2, " for the key grammar")],
]


def grid_reset(pane, cols=89, rows=34, content=None, cursor=None):
    rows_data = []
    for index, runs in enumerate(content if content is not None else TERMINAL):
        if runs:
            rows_data.append({"row": index, "runs": [{"s": s, "x": x} for s, x in runs]})
    return {
        "t": "grid.reset", "pane": pane, "cols": cols, "rows": rows,
        "rows_data": rows_data,
        "cursor": cursor or {"col": 40, "row": 18, "visible": True},
        "links": ["https://herdr.dev"],
    }


CONVO = {
    "t": "convo", "pane": f"{NODE}/w3:p2", "cursor": "c_1", "more": True,
    "turns": [
        {"id": "t_810", "role": "user", "at": "2026-08-20T13:40:02Z",
         "blocks": [{"b": "md", "text": "which key names does herdr actually reject?"}]},
        {"id": "t_812", "role": "assistant", "at": "2026-08-20T13:41:55Z",
         "blocks": [
             {"b": "md", "text": "Six, and they are the ones a phone needs most. I probed the validator directly."},
             {"b": "tool", "name": "Bash", "summary": "probe key grammar", "lines": 48, "state": "done"},
             {"b": "md", "text": "It does not matter, because pane.send_text writes raw bytes."},
             {"b": "code", "lang": "ts", "text": "send(pane, \"\\u001b[5~\")  // PageUp"},
             {"b": "sparkline", "text": "a block type this client has never heard of"},
         ]},
    ],
}

SHELL_HISTORY = [
    "  npm run build",
    "  vite v8.1.0 building for production...",
    "  \u2713 412 modules transformed.",
    "  dist/index.html          0.71 kB",
    "  dist/assets/index.js   184.02 kB",
    "  \u2713 built in 2.41s",
    "",
]


def scrollback(pane, depth=1553, held=90):
    """The node holds `held` rows of a `depth`-row ring; from_top stays absolute."""
    from_top = depth - held
    rows = []
    for i in range(held):
        text = SHELL_HISTORY[i % len(SHELL_HISTORY)]
        if not text:
            continue
        rows.append({"row": from_top + i, "runs": [{"s": 2, "x": text}]})
    return {"t": "scrollback", "pane": pane, "from_top": from_top, "rows": rows,
            "total_rows": held, "complete": False, "capped": True}


# A live shell grid has its prompt at the bottom, with the rows above already filled.
SHELL_GRID = [[] for _ in range(40)]
for _offset, _runs in enumerate([
    [(0, "dbrain@comingclean ~/dev/kampr $ cargo test -p kampr-term")],
    [],
    [(2, "   Compiling kampr-term v0.1.0 (/home/dbrain/dev/kampr/crates/kampr-term)")],
    [(2, "    Finished `test` profile [unoptimized + debuginfo] target(s) in 3.41s")],
    [(2, "     Running unittests src/lib.rs")],
    [],
    [(1, "test result: ok."), (0, " 9 passed; 0 failed; 0 ignored; 0 measured")],
    [],
    [(0, "dbrain@comingclean ~/dev/kampr $ "), (6, " ")],
]):
    SHELL_GRID[31 + _offset] = _runs

PENDING = {
    "t": "pending", "pane": f"{NODE}/w3:p2",
    "question": "Approve edit to server.ts",
    "options": [
        {"key": "1", "label": "Yes"},
        {"key": "2", "label": "Always"},
        {"key": "3", "label": "No"},
    ],
    "source": "screen",
}


class Socket:
    def __init__(self, reader, writer):
        self.reader = reader
        self.writer = writer

    async def send(self, obj):
        payload = json.dumps(obj).encode()
        header = bytearray([0x81])
        length = len(payload)
        if length < 126:
            header.append(length)
        elif length < 65536:
            header.append(126)
            header += length.to_bytes(2, "big")
        else:
            header.append(127)
            header += length.to_bytes(8, "big")
        self.writer.write(bytes(header) + payload)
        await self.writer.drain()

    async def recv(self):
        head = await self.reader.readexactly(2)
        opcode = head[0] & 0x0F
        masked = head[1] & 0x80
        length = head[1] & 0x7F
        if length == 126:
            length = int.from_bytes(await self.reader.readexactly(2), "big")
        elif length == 127:
            length = int.from_bytes(await self.reader.readexactly(8), "big")
        mask = await self.reader.readexactly(4) if masked else b""
        data = await self.reader.readexactly(length)
        if masked:
            data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        if opcode == 8:
            return None
        if opcode != 1:
            return {}
        return json.loads(data.decode())


async def ws_session(sock):
    await sock.send({"t": "hello", "protocol": 1, "node_id": NODE, "node_name": "comingclean",
                     "build": "0.1.0+fixture", "role": "full",
                     "caps": {"push": True, "scrollback": True, "conversation": True,
                              "manage": True, "teleport": True},
                     "security": {"tier": 0, "encrypted": False, "unencrypted_banner": True,
                                  "passkeys": False, "push": False, "installable": False,
                                  "unlocks": ["passkeys", "push", "installable"]}})
    await sock.send(herd())
    await sock.send(STYLES)
    await sock.send({"t": "future.message", "note": "a v1.1 node says things a v1 client ignores"})
    await sock.send({"t": "prefs", "panes": {f"{NODE}/w3:p2": {"zoom": 1.6, "view": "split"}}})
    await sock.send(PENDING)

    prefs = {f"{NODE}/w3:p2": {"zoom": "1.6", "view": "split"}}

    async def reader():
        while True:
            msg = await sock.recv()
            if msg is None:
                return
            kind = msg.get("t")
            if kind == "prefs":
                stored = prefs.setdefault(msg["pane"], {})
                stored.update({k: str(v) for k, v in (msg.get("prefs") or {}).items()})
                await sock.send({"t": "prefs", "panes": prefs})
            elif kind == "ping":
                await sock.send({"t": "pong", "n": msg.get("n", 0)})
            elif kind == "watch":
                pane = msg["pane"]
                if pane == f"{NODE}/w3:p1":
                    await sock.send(grid_reset(pane, cols=94, rows=40, content=SHELL_GRID,
                                               cursor={"col": 33, "row": 39, "visible": True}))
                    if msg.get("scrollback"):
                        await sock.send(scrollback(pane))
                    continue
                await sock.send(grid_reset(pane))
                if pane == f"{NODE}/w3:p2":
                    await sock.send(dict(CONVO, pane=pane))
                    await sock.send(dict(PENDING, pane=pane))

    async def ticker():
        n = 0
        while True:
            await asyncio.sleep(1.0)
            n += 1
            await sock.send({
                "t": "grid.patch", "pane": f"{NODE}/w3:p2",
                "rows": [{"row": 21, "runs": [{"s": 2, "x": f"  watching \u00b7 {n:04d} patches"}]}],
                "cursor": {"col": 40, "row": 18, "visible": n % 2 == 0},
                "links": [],
            })

    await asyncio.gather(reader(), ticker())


async def handle(reader, writer):
    try:
        request = await reader.readuntil(b"\r\n\r\n")
    except Exception:
        writer.close()
        return
    lines = request.decode("latin1").split("\r\n")
    method, path, _ = lines[0].split(" ")
    headers = {}
    for line in lines[1:]:
        if ": " in line:
            k, v = line.split(": ", 1)
            headers[k.lower()] = v

    if headers.get("upgrade", "").lower() == "websocket":
        key = headers["sec-websocket-key"]
        accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
        response = [
            "HTTP/1.1 101 Switching Protocols",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Accept: {accept}",
        ]
        protocol = headers.get("sec-websocket-protocol")
        if protocol:
            response.append(f"Sec-WebSocket-Protocol: {protocol.split(',')[0].strip()}")
        writer.write(("\r\n".join(response) + "\r\n\r\n").encode())
        await writer.drain()
        try:
            await ws_session(Socket(reader, writer))
        except Exception:
            pass
        writer.close()
        return

    body, ctype = serve(path)
    writer.write(
        f"HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {len(body)}\r\n"
        f"Content-Security-Policy: {CSP}\r\n"
        "Cache-Control: no-store\r\n\r\n".encode() + body
    )
    await writer.drain()
    writer.close()


def serve(path):
    if path.startswith("/auth/status"):
        return json.dumps({
            "address": "192.168.1.24:8790", "pairing_code": "7K2-QN9", "https": False,
            "hostname": None, "tier": 0, "webauthn_available": False, "devices": 2,
            "version": "0.1.0+fixture",
        }).encode(), "application/json"
    if path.startswith("/auth/devices"):
        return json.dumps({"devices": [
            {"id": "d1", "name": "this browser", "kind": "browser", "role": "full",
             "last_seen": "now", "current": True},
            {"id": "d2", "name": "Pixel 8", "kind": "phone", "role": "readonly",
             "last_seen": "2h", "current": False},
        ]}).encode(), "application/json"
    name = path.split("?")[0].lstrip("/") or "index.html"
    full = os.path.join(ROOT, name)
    if os.path.isfile(full):
        with open(full, "rb") as handle:
            return handle.read(), mimetypes.guess_type(full)[0] or "application/octet-stream"
    return b"not found", "text/plain"


async def main():
    server = await asyncio.start_server(handle, "0.0.0.0", PORT)
    print(f"fixture node on http://127.0.0.1:{PORT} serving {ROOT}", flush=True)
    async with server:
        await server.serve_forever()


asyncio.run(main())
