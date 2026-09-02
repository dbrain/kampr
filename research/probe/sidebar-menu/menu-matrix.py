#!/usr/bin/env python3
"""Every place a right-click can land in herdr's chrome, and what each one opens.

Second pass of the sidebar probe. Same isolated throwaway session, same 120x40 PTY client, same
persistent VT screen; this one walks a matrix of targets — the sidebar's header, its workspace
rows, its `new` and `menu` buttons, its divider, the agents section, an agent row, empty space, the
tab strip, a pane body and a pane border — right-clicking each from a known screen and recording
the box that appears. It also measures how the menu is dismissed and what the `menu` button opens
with an ordinary left click.

An agent row only exists when herdr has detected an agent, so a `claude` on the pane's PATH that
does nothing but sleep is put there: detection is what is being exercised, not the agent.
"""
import json, os, shutil, socket, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import vt, ptyclient  # noqa: E402

NAME = "kampr-probe-w0-menu"
COLS, ROWS = 120, 40

root = tempfile.mkdtemp(prefix="herdr-probe-menu-")
os.makedirs(root + "/herdr", exist_ok=True)
open(root + "/herdr/config.toml", "w").write(
    "onboarding = false\n\n[experimental]\nallow_nested = true\n")
fakebin = root + "/bin"
os.makedirs(fakebin, exist_ok=True)
with open(fakebin + "/claude", "w") as f:
    f.write("#!/bin/sh\nexec sleep 900\n")
os.chmod(fakebin + "/claude", 0o755)

env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
env.update(XDG_CONFIG_HOME=root, HERDR_CONFIG_PATH=root + "/herdr/config.toml",
           PATH=fakebin + ":" + os.environ.get("PATH", ""))
SOCK = f"{root}/herdr/sessions/{NAME}/herdr.sock"

reading = {"herdr": subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip(),
           "pty": f"{COLS}x{ROWS}", "session": NAME}


def rpc(method, params=None, timeout=10):
    c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    c.settimeout(timeout)
    c.connect(SOCK)
    c.sendall((json.dumps({"id": "p", "method": method, "params": params or {}}) + "\n").encode())
    line = c.makefile().readline()
    c.close()
    return json.loads(line) if line.strip() else None


pid = fd = None
try:
    subprocess.Popen(["herdr", "server", "--session", NAME], env=env, stdin=subprocess.DEVNULL,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 25:
        try:
            rpc("session.snapshot")
            break
        except (OSError, json.JSONDecodeError):
            time.sleep(0.05)

    a = rpc("workspace.create", {"label": "alpha", "cwd": "/tmp", "focus": False})["result"]
    rpc("workspace.create", {"label": "bravo", "cwd": "/tmp", "focus": False})
    rpc("tab.create", {"workspace_id": a["workspace"]["workspace_id"], "label": "second",
                       "cwd": "/tmp", "focus": False})
    rpc("pane.split", {"target_pane_id": a["root_pane"]["pane_id"], "direction": "right",
                       "focus": False})

    pid, fd = ptyclient.spawn(["herdr", "--session", NAME], COLS, ROWS, env)
    sc = vt.Screen(COLS, ROWS)

    def settle(secs=1.2):
        data = ptyclient.drain(fd, secs)
        vt.feed(sc, data.decode("utf-8", "replace"))
        return sc.text(), data

    settle(4.0)
    os.write(fd, b"claude\n")
    settle(4.0)
    snap = rpc("session.snapshot")["result"]["snapshot"]
    reading["agentsDetected"] = snap["agents"]
    base, _ = settle(2.0)
    reading["baseline"] = base
    width = snap["layouts"][0]["area"]["x"]
    text_width = width - 1
    reading["sidebarWidth"] = width

    rows = base.split("\n")

    def sgr(b, col, row, rel=False):
        return f"\x1b[<{b};{col};{row}{'m' if rel else 'M'}".encode()

    def reset():
        os.write(fd, b"\x1b")
        settle(0.6)

    def box(before, after):
        b, a = before.split("\n"), after.split("\n")
        return [f"{i:2d}|{a[i]}" for i in range(len(a)) if a[i] != b[i]]

    def press(button, col, row, secs=1.2):
        before = sc.text()
        os.write(fd, sgr(button, col, row))
        time.sleep(0.08)
        os.write(fd, sgr(button, col, row, rel=True))
        after, data = settle(secs)
        return {"at": [col, row], "bytes": len(data), "changed": after != before,
                "changedRows": box(before, after)}

    def find(needle, in_sidebar=True):
        for i, r in enumerate(rows):
            hay = r[:text_width] if in_sidebar else r
            j = hay.find(needle)
            if j >= 0:
                return i + 1, j + 2
        return None

    targets = {}
    for label, needle, side in (
        ("sidebarHeaderSpaces", "spaces", True),
        ("workspaceRowAlpha", "alpha", True),
        ("workspaceRowBravo", "bravo", True),
        ("sidebarNewButton", "new", True),
        ("sidebarMenuButton", "menu", True),
        ("agentsHeader", "agents", True),
        ("agentsGroupedToggle", "grouped", True),
    ):
        hit = find(needle, side)
        if hit:
            targets[label] = hit
    agents_at = next((i for i, r in enumerate(rows)
                      if r[:text_width].strip().startswith("agents")), None)
    if agents_at is not None:
        below = [i for i, r in enumerate(rows[agents_at + 1:], agents_at + 1)
                 if r[:text_width].strip()]
        for n, i in enumerate(below[:2]):
            r = rows[i][:text_width]
            targets[f"agentsSectionRow{n}"] = (i + 1, len(r) - len(r.lstrip()) + 2)
    blank = [i for i, r in enumerate(rows) if not r[:text_width].strip()]
    targets["emptySidebarSpaceAboveButtons"] = (blank[len(blank) // 3] + 1, 4)
    targets["emptySidebarSpaceAtFoot"] = (blank[-1] + 1, 4)
    divider = [i for i, r in enumerate(rows) if r[:text_width].startswith("─")]
    if divider:
        targets["sidebarDivider"] = (divider[0] + 1, 4)
    for label, needle in (("tabStripTabOne", " 1 "), ("tabStripTabSecond", "second"),
                          ("tabStripPlus", "+")):
        j = rows[0].find(needle, width)
        if j >= 0:
            targets[label] = (1, j + 2)
    targets["paneBody"] = (ROWS - 6, COLS - 25)
    targets["paneBorder"] = (2, width + 1)

    reading["targets"] = targets
    shots = {}
    for label, (row, col) in targets.items():
        reset()
        shots[label] = press(2, col, row)
    reading["rightClicks"] = shots

    # Dismissal, on the workspace menu, three ways.
    # The workspace menu's own middle row is the marker: "Rename" alone also matches the pane
    # menu's "Rename pane", which is the thing a right-click elsewhere replaces it with.
    MARK = "\u2502 Rename     \u2502"
    dismiss = {}
    row, col = targets["workspaceRowAlpha"]

    def open_workspace_menu():
        reset()
        shot = press(2, col, row)
        return MARK in sc.text(), shot

    for label, keys in (("escape", b"\x1b"), ("enterOnFirstItem", b"\r"), ("q", b"q")):
        opened, _ = open_workspace_menu()
        os.write(fd, keys)
        after, _ = settle(1.2)
        dismiss[label] = {"opened": opened, "menuStillDrawn": MARK in after,
                          "screen": [f"{i:2d}|{r}" for i, r in enumerate(after.split("\n"))
                                     if r[:text_width].strip() or "\u2502 " in r[:width + 20]][:8]}
        reset()
        os.write(fd, b"\x1b")
        settle(0.6)

    for label, button in (("leftClickOutside", 0), ("rightClickOutside", 2)):
        opened, _ = open_workspace_menu()
        os.write(fd, sgr(button, COLS - 20, ROWS - 5))
        time.sleep(0.08)
        os.write(fd, sgr(button, COLS - 20, ROWS - 5, rel=True))
        after, _ = settle(1.2)
        dismiss[label] = {"opened": opened, "workspaceMenuStillDrawn": MARK in after,
                          "paneMenuNowDrawn": "Close pane" in after}
        reset()

    # Arrow keys over the open menu: does the highlight move, or does the pane get the key?
    opened, _ = open_workspace_menu()
    before = sc.text()
    os.write(fd, b"\x1b[B")
    after, _ = settle(1.0)
    dismiss["downArrow"] = {"opened": opened, "menuStillDrawn": MARK in after,
                            "screenChanged": after != before}
    reset()
    reading["dismissal"] = dismiss

    # Activating an item: a left click on the menu's own row.
    activate = {}
    row, col = targets["workspaceRowBravo"]
    reset()
    press(2, col, row)
    activate["openedAt"] = [col, row]
    activate["afterLeftClickOnRename"] = press(0, col + 3, row + 1, 1.5)["changedRows"][:10]
    os.write(fd, b"\x1b")
    settle(0.8)
    reset()
    press(2, col, row)
    activate["afterLeftClickOnClose"] = press(0, col + 3, row + 2, 1.8)["changedRows"][:10]
    reset()
    reading["activation"] = activate

    # What the sidebar's own `menu` button opens with an ordinary left click, and what `new` does.
    left = {}
    for label in ("sidebarMenuButton", "sidebarNewButton", "workspaceRowBravo"):
        if label not in targets:
            continue
        reset()
        row, col = targets[label]
        left[label] = press(0, col, row, 1.5)
        reset()
    reading["leftClicks"] = left
finally:
    if fd is not None:
        try:
            os.write(fd, b"\x02q")
            time.sleep(0.5)
            os.close(fd)
        except OSError:
            pass
    if pid:
        try:
            os.kill(pid, 9)
        except OSError:
            pass
    subprocess.run(["herdr", "session", "stop", NAME], env=env, capture_output=True)
    time.sleep(0.5)
    subprocess.run(["herdr", "session", "delete", NAME], env=env, capture_output=True)
    shutil.rmtree(root, ignore_errors=True)

print(json.dumps(reading, indent=2))
