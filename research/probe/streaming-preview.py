#!/usr/bin/env python3
"""What the live preview is lifted from while Claude streams a message.

The node reads the message a harness is painting off the visible screen (`kampr_journal::live`),
ships it as one markdown block under the reserved `live` id, and revises it as the text grows. This
dumps the rows that reader walks — the `●` head and its indented continuations at the foot of the
screen — every 300 ms through a real streamed turn, so what the client is asked to render as
markdown can be read rather than guessed at: the wrapping, the harness's own glyphs, and whether a
code fence is ever open at the moment a frame goes out.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from rpc import rpc
from importlib import import_module

composer = import_module("composer-line")

HOME = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
NAME = f"kampr-probe-stream-{os.getpid()}"
SOCK = os.path.join(HOME, "herdr", "sessions", NAME, "herdr.sock")
composer.NAME = NAME
composer.SOCK = SOCK
MESSAGE = '●'
PROMPT = '❯'


def call(method, params=None):
    r = rpc(method, params or {}, sock_path=SOCK)
    if not r or "error" in r:
        raise SystemExit(f"{method} failed: {r}")
    return r["result"]


def send(pane, text):
    call("pane.send_text", {"pane_id": pane, "text": text})


def preview(rows):
    """The same walk `kampr_journal::live::read` makes: up from the composer, gathering the
    indented body until the row that opens the block."""
    end = max((i for i, r in enumerate(rows) if r.startswith(PROMPT)), default=len(rows))
    body, head, clipped = [], None, True
    for line in reversed(rows[:end]):
        if line.startswith(MESSAGE):
            head, clipped = line[1:].strip(), False
            break
        blank = not line.strip()
        continuation = line.startswith("  ")
        if not blank and not continuation:
            if not body:
                continue
            return None, clipped
        if blank and not body:
            continue
        body.append(line)
    text = "\n".join(([head] if head else []) + [b[2:].rstrip() for b in reversed(body)])
    return (text.strip() or None), clipped


def main():
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-stream-XXXXXX"], capture_output=True, text=True).stdout.strip()
    composer.start()
    try:
        ws = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        pane = ws["root_pane"]["pane_id"]
        watch = composer.Watch(pane)
        time.sleep(1.0)
        send(pane, "claude --model haiku\r")
        time.sleep(12)
        if any("trust" in r.lower() for r in watch.look()[0]):
            send(pane, "1")
            time.sleep(6)

        send(
            pane,
            "write me a short markdown answer with a heading, a bulleted list of three items, "
            "one **bold** phrase, and a fenced bash code block. no tools.",
        )
        send(pane, "\r")

        seen = []
        end = time.time() + 60
        while time.time() < end:
            rows, _, _ = watch.look()
            text, clipped = preview(rows)
            if text and (not seen or seen[-1] != text):
                seen.append(text)
                print(f"\n--- revision {len(seen)}  clipped={clipped}  {len(text)} chars")
                for line in text.splitlines():
                    print(f"    {line!r}")
            if any("esc to interrupt" not in r.lower() for r in rows) and len(seen) > 3 and text is None:
                break
            time.sleep(0.3)
        print(f"\n{len(seen)} distinct previews")
        for _ in range(3):
            send(pane, "\x03")
            time.sleep(0.3)
        watch.close()
    finally:
        composer.stop()
        shutil.rmtree(cwd, ignore_errors=True)


if __name__ == "__main__":
    main()
