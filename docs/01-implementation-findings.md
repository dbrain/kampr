# Kampr — implementation findings

What Herdr actually exposes, what Collie actually does, and which of Kampr's goals those two facts
make possible, conditional, or impossible.

Everything marked **[probed]** was measured on this machine on **2026-08-20** against
**herdr 0.8.2, protocol 20**, using `research/probe/rpc.py` (raw socket JSON-RPC) and
`research/probe/ptyclient.py` (PTY-hosted Herdr clients). Probes ran in throwaway named sessions
(`kp2`…`kpb`) and a throwaway workspace, all torn down afterwards. Everything marked **[docs]** comes
from Herdr's own docs, mirrored under `research/`.

---

## 0. The one finding that changes the design

Herdr 0.8.2 ships a **first-class terminal frame stream for third-party bridges**:

```bash
herdr terminal session observe w1:p1 --cols 120 --rows 40   # read-only, many at once
herdr terminal session control w1:p1 --cols 120 --rows 40   # read/write, one owner
```

Both emit newline-delimited JSON frames of base64 ANSI, **each consumer at its own requested
geometry**, with cursor state included, at ~27 ms median echo latency. Collie does not use this —
its [ADR 0008] explicitly refuses to run a terminal emulator, and that ADR's own text says
`observe`/`control` were *"unprobed"*. They are now probed, and they answer most of what Kampr wants.

The whole "Collie just wraps text" complaint is a *consequence of a deliberate architectural
decision Collie made*, not a limitation of Herdr. Kampr can take the opposite bet.

[ADR 0008]: research-notes — `.adr/0008-collie-does-not-run-a-terminal-emulator.md` in the Collie checkout

---

## 0.5 Decisions locked

- **Take the opposite bet to Collie.** Stream real terminal frames and render them with a real
  emulator. Collie's ADR 0008 is a coherent position for Collie's product; it is the source of every
  complaint that motivated Kampr.
- **Soft native is the shipping look**, built on a token layer (colour, type, radius, border) so
  phosphor / warm editorial / brutalist stay reachable as themes rather than rewrites. The design
  canvas carries all four as a live switch, which is the proof it holds.
- **Kampr never resizes a session.** Desktop geometry is authoritative and permanent. Small screens
  are handled by zoom, pan and the Conversation view — never by reshaping the pane. §3.4.
- **Rust + axum on the server, Kotlin Multiplatform + Compose Multiplatform on the clients**, with VT
  emulation done once on the server and a cell-grid protocol to the clients. §5.5.

---

## 1. Herdr's three extension planes

Herdr is not one API. It's three, with different guarantees.

| Plane | Transport | Stability | What it's for |
|---|---|---|---|
| **Socket API** | newline-delimited JSON over `~/.config/herdr/herdr.sock` | versioned (`protocol: 20`), schema-published | herd state, structural ops, input, one-shot reads |
| **Terminal streams** | NDJSON over a CLI child process' stdio | documented for "third-party bridges", no protocol version of its own | live per-pane rendering + input |
| **Plugin host** | `herdr-plugin.toml` + argv commands | v1, documented | installation, actions, panes, event hooks, keybindings |

There is a fourth, `~/.config/herdr/herdr-client.sock` — **do not use it**. **[probed]** It is
bincode-framed (the binary links `bincode`, and garbage JSON gets an immediate RST), carries the
private `SemanticFrame` / `TerminalAnsi` encodings selected by `HERDR_RENDER_ENCODING`, and its
`FrameData` type has `cells` / `fg` / `bg` / `hyperlink` fields. It is exactly the structured grid a
web client would want, and it is exactly the thing that will break on every Herdr release.

### 1.1 Socket API — 91 methods **[probed]**

Full generated catalogue with parameter shapes: `research/herdr-methods.md`. Raw schema:
`research/herdr-api-schema.json` (`herdr api schema --json`).

Shape confirmations that matter:

- One request per connection; the server closes after the single response. `events.subscribe` is the
  only streaming exception. `id` must be a string.
- `session.snapshot` returns workspaces + tabs + panes + agents + layouts + focus in one round trip.
- **`revision` is live again.** Collie's `HERDR_API.md` records it as a permanent `0` stub on 0.7.x.
  On 0.8.2 panes report real revisions **[probed]**, so it is usable as a cheap change detector.

### 1.2 Events — 26 kinds, 27 subscribable, **none of them "output changed"** **[probed]**

Subscribable types (from the validator's own error message):

```
workspace.created/updated/metadata_updated/renamed/moved/reordered/closed/focused
worktree.created/opened/removed
tab.created/closed/focused/renamed/moved
pane.created/closed/updated/focused/moved/exited
pane.agent_detected  pane.output_matched  pane.agent_status_changed  pane.scroll_changed
layout.updated
```

`pane_output_changed` **exists as an event kind in the schema but is not subscribable** — the
subscription enum rejects it. **[probed]** `pane.updated` fires on subscribe and on structural
changes, *not* on output; a 3-second shell loop producing output emitted zero events.

**Consequence:** if you build on the JSON API alone you are forced into polling, exactly like Collie.
`pane.wait_for_output` is not a rescue: it matches against *current* content and returns immediately
if already present (`regex: "."` returns in 0.00 s), so it is a match-waiter, not a change-waiter.
**[probed]** It does detect genuinely new content in ~110 ms, and helpfully returns the pane read
alongside the match.

### 1.3 `pane.read` fidelity **[probed]**

`{pane_id, source: visible|recent|recent_unwrapped|detection, lines, format: text|ansi, strip_ansi}`

- `format: "ansi"` **preserves bold, italic, underline, reverse, blink, 256-colour and 24-bit
  truecolour**, re-serialised per cell-run with a leading `ESC[0m`.
- **OSC 8 hyperlinks are lost** — a `\e]8;;url\e\\LINK\e]8;;\e\\` round-trips as the bare text `LINK`.
- `strip_ansi` is **ignored** when `format: "ansi"`.
- Collie's warning still holds: `source: "recent"` with `lines > viewport_rows` physically scrolls the
  operator's pane. Background polling must use `visible`.

### 1.4 Key grammar — still no Home/End/PgUp/PgDn on 0.8.2 **[probed]**

`pane.send_keys` accepts, case-insensitively:

- `Up` `Down` `Left` `Right` `Tab` `Enter` `Escape` `Space` `Backspace` / `BS`
- `F1`…`F12` — and **`F13` is accepted too**, undocumented
- any single character (`a`, `1`, `/`, `€`)
- modifier chords in any order: `ctrl+c`, `shift+tab`, `alt+Up`, `ctrl+alt+shift+p`

Rejected with `invalid_key`: **`PageUp` `PageDown` `Home` `End` `Insert` `Delete` `BackTab`
`ScrollLock` `NumLock` `PrintScreen` `Pause` `Menu` `CapsLock`**, and tmux spellings (`C-c`, `BTab`).

**The workaround is total.** `pane.send_text` writes **raw bytes** to the PTY — verified by sending
into `cat -v`, which echoed `^[`, `^[[5~`, `^[[6~`, `^[[H`, `^[[F`, `^[[3~`, `^A`, and UTF-8
(`→ ✓ é`) intact. **[probed]** So every key Herdr's validator refuses can be sent as its escape
sequence. There is no key Kampr cannot deliver.

### 1.5 Geometry is a **shared, last-writer-wins** global **[probed]**

This is the single nastiest constraint in the system.

- A pane's PTY size comes from the attached TUI client's window. `pane.resize` is *layout* resizing
  inside a split, not PTY sizing — on a single-pane tab it returns `{changed: false}`.
- With two clients attached at 100×30 and 60×20, the pane became **18 rows** (the newcomer won).
  Resizing the first client to 200×50 took it back to **50 rows** while the small client was still
  attached. **Last writer wins; there is no smallest-wins negotiation.**
- With no client attached, panes keep the last client's geometry.

So two people (or one person on two devices) using the same Herdr session in *attach* mode fight over
the grid. Collie sidesteps this by never owning geometry; the cost is that a phone gets a
desktop-width grid, which is Collie's issue #23 and #53 and the root of the "just wraps text" feel.

### 1.6 Terminal streams — the good part **[probed]**

`herdr terminal session observe|control <pane-or-agent> [--cols N] [--rows N]`, plus
`herdr terminal attach <terminal_id> [--takeover]`. **[docs]** These are explicitly blessed:
*"For third-party bridges that only need rendered terminal bytes, use a read-only terminal session
observer."*

**Frames out (stdout, one JSON object per line):**

```jsonc
{"type":"terminal.frame","seq":1,"width":60,"height":20,"encoding":"ansi","full":true,
 "bytes":"<base64 ANSI>"}
{"type":"terminal.closed","reason":"detached"}
```

- First frame is `full: true`; subsequent frames are **diffs** (cursor-addressed partial repaints).
- A resize produces a fresh `full: true` frame at the new size.
- `reason` values seen: `detached` (after `terminal.release`), `terminal attach taken over`,
  `terminal attach failed: terminal … already has an attached client; retry with --takeover`.

**Commands in (stdin, control mode only) — enumerated from the validator's own errors:**

| Command | Fields |
|---|---|
| `terminal.input` | `text` (string) **or** `bytes` (base64) |
| `terminal.resize` | `cols`, `rows` (u16) |
| `terminal.scroll` | `direction`: `"up"`\|`"down"`, `lines` (u16) |
| `terminal.release` | — |

`observe` **ignores stdin entirely** — no error, no effect. It is genuinely read-only.

**Measured properties:**

| Property | Result |
|---|---|
| Cursor position in frames | **Yes** — frames end with `ESC[row;colH` + `ESC[?25h/l` |
| Synchronised output | Yes — every frame is wrapped in `ESC[?2026h … ESC[?2026l` |
| Hyperlink modelling | Frames emit `ESC]8;;ESC\` resets, so OSC 8 appears to be tracked (a live hyperlink round-trip is **unverified**) |
| Multiple observers | **Yes** — two observers at 80×24 and 40×12 streamed simultaneously, each at its own size |
| Observer + controller together | **Yes** — both streamed concurrently |
| Observer effect on shared PTY | **None** — PTY stayed 36 rows while an observer ran at 20 |
| `observe` with **no** `--cols/--rows` | Defaults to **120×40**, not the pane's native size. PTY untouched. To mirror faithfully you must pass the pane's real geometry |
| `control` with **no** `--cols/--rows` | Also defaults to 120×40 — **and resizes the PTY to it**. There is no flag to decline geometry: control *always* claims it |
| Desktop resize while a controller holds | **Ignored.** Desktop client went to 120×44; PTY stayed at the controller's 40 until release |
| Controller effect on shared PTY | **Resizes it** — PTY went 36 → 20 → 30 as the controller resized |
| **Is that resize reversible?** | **Yes, automatically.** With a desktop client attached at 100×30, a controller at 52×20 narrowed the PTY to 20; `terminal.release` restored it to 30. So did `SIGKILL` on the controller, **within 1 s**. No flapping while the lease is held (0 frames over 3 idle seconds) |
| Lease held by a *frozen* controller | **Held indefinitely** — `SIGSTOP`'d controller kept the pane at 16 rows; the socket is still open, so Herdr has no reason to reclaim. **This is the one case Kampr must time out itself** |
| Controller arbitration | One owner; second gets `terminal.closed`; `--takeover` evicts the incumbent, who receives `reason: "terminal attach taken over"` |
| Echo latency (control, local) | **p50 27 ms, p90 98 ms, min 17 ms** over 10 keystrokes |
| Burst bandwidth | `seq 1 20000` → **3 frames, 1.9 KB total** — Herdr coalesces to grid state, it does not replay the byte stream |
| Works fully headless | **Yes** — server started with `herdr server --session X`, workspace created over the API, observed, with no TUI client ever attached |

**The one real limitation:** `observe --cols` **crops, it does not reflow.** A 120-column line on a
93-column grid observed at 60 columns showed grid-row-1 truncated at column 60 — columns 61–93 are
simply absent, not wrapped to the next row. **[probed]** Only `control`'s `terminal.resize` produces a
genuine narrow reflow, because it resizes the real PTY.

### 1.7 Named sessions = independent servers **[probed]**

`herdr --session <name>` / `herdr session attach|list|stop|delete <name>`. Each gets its own socket
tree at `~/.config/herdr/sessions/<name>/{herdr.sock,herdr-client.sock,session.json}` and its own
panes, tabs, workspaces — sharing only the global `config.toml`. Socket resolution order **[docs]**:
`--session` → `HERDR_SOCKET_PATH` → `HERDR_SESSION` → default.

This is Kampr's escape hatch from §1.5: a session nobody attaches a desktop TUI to has no geometry
competitor.

### 1.8 Remote attach is a *client*, not a server API **[docs]**

`herdr --remote workbox [--session agents]` makes the **local** Herdr a thin SSH client of a remote
Herdr server. It streams a UI to a local terminal. There is no cross-host socket API, no remote JSON
endpoint, no federation. **Multi-host is entirely Kampr's problem to solve.**

### 1.9 Plugin host **[docs]**

Manifest sections: `[[build]]` (GitHub install only, not `link`), `[[startup]]` (one-shot, per
enabled plugin, after session restore and again after live handoff), `[[actions]]`, `[[events]]`
(`on = "<event>"`), `[[panes]]`, `[[link_handlers]]`, plus `[[keys.command]]` bindings in
`config.toml`.

Pane placements: `overlay` (default), `popup`, `split`, `tab`, `zoomed`. **`popup` accepts `width`
and `height`** as cell counts or `"80%"` strings, is session-modal, receives all terminal input
including Escape, and closes when its command exits. **That is the setup-wizard vehicle.**

Injected env: `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`, `HERDR_PLUGIN_ID`, `HERDR_PLUGIN_ROOT`,
`HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_STATE_DIR`, `HERDR_PLUGIN_CONTEXT_JSON`, plus
`HERDR_WORKSPACE_ID` / `HERDR_TAB_ID` / `HERDR_PANE_ID` where available.

Constraints worth designing around:

- **No `plugin update` in v1** — refresh is a reinstall, which replaces the checkout but does **not**
  restart anything you started. Collie hand-rolls an `update` action for exactly this; Kampr must too.
- **No plugin storage API** — own your files under `HERDR_PLUGIN_STATE_DIR`.
- **No sandbox.** Plugin code runs as you, with your environment, with full CLI access.
- **`[[startup]]` hooks are one-shot, not supervised.** A long-lived daemon still needs
  systemd/launchd, but a startup hook is a perfectly good "make sure it's running" nudge — which is
  strictly better than Collie's "systemd only, and Herdr never nudges it".

### 1.10 Agent identity and transcripts **[probed]**

`pane.agent_session` is `{source, agent, kind: "id"|"path", value}`. Claude and Codex report
`kind: "id"` (a UUID); pi reports `kind: "path"`. Herdr keeps reporting the **last** session announced
for a pane, so a relaunched pane can advertise a stale harness — compare against the pane's own
`agent` field before trusting it.

`IntegrationTarget` enumerates the 17 harnesses Herdr can install hooks for: `pi`, `omp`, `claude`,
`codex`, `copilot`, `devin`, `droid`, `kimi`, `opencode`, `kilo`, `hermes`, `qodercli`, `qwen`,
`cursor`, `mastracode`, `antigravity_cli`, `grok`.

**On-disk transcripts exist and contain raw markdown.** `~/.claude/projects/<slug>/<uuid>.jsonl` on
this machine parses to `{assistant: 676, user: 335, system: 29, …}` records whose assistant text is
literal markdown (`"the smart move is **OpenSCAD**, not a CAD GUI"`). Codex writes
`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. **[probed]**

This is the *only* route to real markdown/table rendering — see §3.6.

---

## 2. What Collie is, and why it doesn't fit

Collie 0.27.0, MIT, `AltanS/collie`. Bun bridge (`bridge/`) + Vite/React 19/Tailwind 4 PWA (`web/`),
behind `tailscale serve`, run as a `systemd --user` service with a thin Herdr plugin as launcher.

It is a well-argued piece of engineering. Its architecture documents are worth reading before writing
a line of Kampr — `ARCHITECTURE.md`, `HERDR_API.md`, and ten ADRs. The mismatch is not sloppiness; it
is that Collie optimises for a different product.

### 2.1 Collie's five load-bearing decisions

1. **"Deliberately not full terminal mirroring."** The product is *triage*: a NEEDS-YOU list,
   push notification, tap, structured prompt blocks, explicit Send. The pane text is context below the
   fold.
2. **No terminal emulator anywhere** (ADR 0008) — not in the browser, not in the bridge. The client
   contract is `StyledLine[]`; xterm.js is "refused outright". Reasons given: Herdr already rendered
   the grid; a second renderer disagrees with the first; 34 byte-faithful fixtures and 1,757 lines of
   grammar tests are pinned to `pane.read` output; and `control` "means Collie fights the person at
   the desk".
3. **Poll, don't stream** — `session.snapshot` on a timer, event-poked, with the snapshot always
   authoritative. The browser polls the bridge too. No WebSocket anywhere.
4. **Explicit Send, never live keys** — a text box plus quick actions plus a special-key strip, then a
   Send button. Deliberate: it makes dictated voice input reviewable.
5. **Auth is delegated to the front door** — `tailscale serve` terminating TLS and injecting
   `Tailscale-User-Login`, trusted only from loopback; or a reverse proxy asserting
   `COLLIE_DEVICE_HEADER`. Collie explicitly says device ids "are names your proxy asserts, not
   secrets — treat them as guessable". **There is no login, no credential, no session.** `tailscale
   funnel` is explicitly forbidden.

### 2.2 Where each Kampr goal collides

| Kampr wants | Collie's position |
|---|---|
| Multiple / meshed remotes | Single host, single socket, single front door. Not a gap — a scope boundary. |
| Built-in auth good enough to expose publicly | Explicitly out of scope; the tailnet *is* the auth. Public exposure is documented as forbidden. |
| Full-screen pretty desktop UI | Mobile-first PWA; desktop is the mobile layout on a big screen. |
| Live key entry | Refused by design (#4 above). |
| JuiceSSH-style key row | Partially there as a "special-key strip", but it emits `pane.send_keys`, so it inherits §1.4's missing Home/End/PgUp/PgDn. |
| Markdown/table rendering of agent output | ADR 0008: *"A TUI paints cells; it does not paint structure."* Collie renders wrapped rows. It reads transcripts separately (`bridge/journal/`) but as turn text, not as a rendered replacement for the mirror. |
| Scrollback | Impossible in Collie's mirror (alt-screen panes have no scrollback ring via `pane.read`); it reads the agent's transcript file instead. |

**Verdict:** none of these are Collie bugs to be fixed. They are the product. Kampr is a different
product, and should be a separate plugin rather than a fork.

---

## 3. Goal-by-goal: possible, conditional, impossible

### 3.1 Multiple remotes, meshed — **POSSIBLE, and entirely ours to build**

Herdr contributes nothing here (§1.8) and blocks nothing. The shape that falls out of the constraints:

- A **kampr node** runs on every host, next to that host's Herdr server(s). It is the only thing that
  touches a Unix socket or spawns `herdr terminal session …`.
- Any node can run in **hub** mode: it holds authenticated outbound connections from peer nodes (or
  dials them), and presents the union as one herd. Nodes are peers; "hub" is a role, not a build.
- Because nodes connect *outbound* to a hub, only the hub needs to be reachable — which is what makes
  a laptop behind NAT joinable.
- A node enumerates **every named session on its host** (`herdr session list --json`), not just the
  default. Multi-session is free multi-remote on one box.

Cost to be honest about: frames now cross two hops, so §1.6's 27 ms becomes 27 ms + WAN RTT. That is
still a normal SSH-feel terminal.

### 3.2 Built-in auth — **POSSIBLE, mandatory, and tiered**

The Herdr socket is filesystem-permission-scoped to your uid and that is the entire security model.
Anything reaching Kampr can type into live terminals — unrestricted RCE on every meshed host — so the
auth has to be real, not header-shaped. Collie's `Tailscale-User-Login` model is safe only because it
is loopback-plus-tailnet; "make it public" deletes that precondition.

But §3.7 shows auth cannot be one thing, because passkeys need a domain:

- **Tier 0** (IP + port): pairing code → device-bound bearer token, LAN bind only, persistent
  "unencrypted" banner, expiry that forces a deliberate decision.
- **Tier 1+** (hostname + cert): **WebAuthn passkeys as the primary credential** — phone biometrics,
  nothing to type, nothing to leak, phishing-resistant. Recovery code printed once.

Shared across every tier:

- **Device-bound sessions.** Enrolment mints a long-lived, revocable, per-device token; the device
  list is visible and each entry killable.
- **The bridge authenticates in-process.** A reverse proxy supplies TLS and nothing else; header
  trust is opt-in via `trust_proxy`, never assumed.
- **Node-to-node auth is separate** from user auth (mutual token or mTLS), so a compromised viewer
  session cannot impersonate a node.
- Rate limiting, per-device audit log (Collie's `audit.ts` is the pattern), and a read-only role for
  devices you half-trust.

### 3.3 Setup — **POSSIBLE, and it should be a ladder rather than a wizard**

The goal is "runs immediately, everything else optional". So the first run is not a six-step gate: the
node starts, binds to the LAN, prints a URL and a pairing code, and works. Setup is then a screen of
labelled upgrades, each stating what it unlocks (§3.7's table is exactly that content).

Two surfaces:

- **In-terminal**: a `[[panes]]` entry with `placement = "popup"`, `width = "80%"`. Session-modal,
  receives Escape, closes when the command exits. Reachable from `[[actions]]` and a `prefix+…` bind.
- **In-browser**: the same ladder, for people who start from the phone.

Both must also cover what Herdr will not do for us: **supervision** (`[[startup]]` hooks are one-shot,
so a systemd/launchd unit is still required) and **update** (plugin v1 has no `plugin update`, and a
reinstall restarts nothing).

### 3.4 Responsive UI — **Kampr never resizes a session**

This reversed twice, and the probes are why. The final position is the simplest one available.

`terminal session control` **always claims geometry** — with no `--cols/--rows` it still takes the PTY,
to a 120×40 default — and while it holds, the desktop client's own resize is **ignored** **[probed]**.
There is no read-only-input control mode. So Kampr does not use control mode at all.

What it uses instead, both of which leave the PTY alone:

- **`observe --cols <native> --rows <native>`** for pixels. Native geometry comes from
  `session.snapshot`'s `layouts[].panes[].rect`; a `layout.updated` subscription tells us when the desk
  resizes, and the observer restarts to match (one `full` frame, ~2 KB).
- **`pane.send_text` / `pane.send_keys`** for keystrokes. One-shot JSON-RPC, no ownership, no session
  state. Several viewers can type at once, which is correct for a personal tool.

**The pane stays whatever shape the desktop made it, permanently, for everyone.** Kampr is structurally
incapable of reshaping a session — not by policy, by construction.

That turns the small-screen problem into a rendering problem, which is where it belongs:

| Surface | How it reads |
|---|---|
| **Landscape** | 844 px fits 94 columns at 13 px. Nothing to zoom, nothing to pan |
| **Conversation** (agent panes, the default) | reflows natively — no grid involved at all |
| **Portrait terminal** | pinch-zoom and pan over the real grid, zoom remembered per pane per device, follow-cursor keeps the caret in view |

The terminal surface **always fills the viewport**. Scrollback and the live grid are one continuous
scroll, so the space above a short grid carries history rather than nothing, and the default zoom
fills at least one axis instead of fitting inside both. A pane with no ring pans horizontally instead.
No client-side row cap: the node's ring bound is a memory limit and is configurable.

**Scrollback — the answer is better than "no `terminal.scroll`" suggested.** Frames cannot supply it:
`seq 1 200` on a 30-row pane put only 29 distinct lines across the *entire* frame stream — the final
viewport. Lines 1–171 were never transmitted **[probed]**. A frame-fed emulator therefore cannot
rebuild history.

But Herdr keeps the ring and hands it over directly:

| Pane kind | `max_offset_from_bottom` | `pane.read recent` | History from |
|---|---|---|---|
| Shell / normal screen | > 0 (171 after 200 lines) | **0.002 s for 401 lines, viewport unmoved**; `format:"ansi"` keeps all 256-colour SGR at 11.7 KB for 400 lines | the ring, directly |
| Alt screen (Claude, Codex, vim) | **0** | degrades to the viewport, instantly, unmoved | the transcript |

So **the node holds the scrollback**: backfill from `pane.read recent format=ansi` on watch, run it
through the same emulator so styling matches the live grid, and extend it as the ring grows.
Over-asking clamps harmlessly (`lines: 5000` returns 1000, `truncated: true` — see #51, which corrected #29) **[probed]**.

**The interlock** — read scrollback only when `max_offset_from_bottom > 0` **and** the pane has no
detected `agent`. The second condition is Collie's documented hazard: on an idle *recognised agent*
pane, `recent` with `lines > viewport_rows` harvests through the agent's own mouse-scroll interface,
which is slow and visibly moves the operator's screen. Encoded as
`Pane::scrollback_is_safe_to_read` in `crates/kampr-herdr`.

Agent panes lose nothing by this: they are alt-screen, so no ring exists to miss, and the
Conversation view is a better history than a ring — whole-session, searchable, structured.

### 3.5 Live keys + JuiceSSH key row — **POSSIBLE, fully**

`terminal.input` takes `text` or base64 `bytes`, so every keystroke goes through live, and any byte
sequence is legal. The key row is then a pure UI problem:

- Modifier latches (Ctrl/Alt/Shift/Fn) that arm the next tap, exactly as in the screenshot.
- Direct keys — Esc, Tab, `/`, `|`, `-`, arrows, Home, End, PgUp, PgDn, Del, F-keys — emitted as raw
  escape sequences, which sidesteps §1.4's rejected key names entirely.
- Native soft keyboard for text, so dictation keeps working for free (Collie's best insight).
- In **observe** mode the same row still works via `pane.send_text` / `pane.send_keys` — the JSON API
  accepts input regardless of who owns the terminal stream. Reading and writing are independent.

### 3.6 Markdown / table rendering of Claude & Codex output — **CONDITIONAL, and the answer is two views**

ADR 0008 is right about the hard part: `pane.read` and `terminal.frame` both come from a renderer, so
the structure is already gone. A markdown table has become box-drawing characters in cells. **You
cannot recover markdown from the terminal stream.** Attempting it means a heuristic re-parser that
will be wrong in ways that are hard to notice.

But you do not have to. §1.10: the transcripts are on disk, in the harness's own format, with the
original markdown intact. So Kampr ships **two views of the same pane**:

- **Terminal view** — the node's own VT emulation over the frame stream, drawn as a cell grid (§5.5). Truthful, live, interactive, exactly what the
  desk sees. This is where you type.
- **Conversation view** — parsed from the transcript file, rendered as real markdown: tables as
  tables, code blocks with syntax highlighting and a copy button, diffs as diffs, tool calls
  collapsible. Read-optimised, phone-shaped, scrollable through the entire session rather than one
  viewport.

`pane.agent_session` links the two, and Collie's `bridge/journal/` (adapters for claude, codex,
opencode, pi behind a registry keyed on the agent name) is the right shape to reimplement.

Where this is genuinely limited: a harness with no adapter gets terminal view only, and the
conversation view is always slightly behind the live screen because it reflects flushed transcript
records. Both are acceptable; neither is hidden from the user.

### 3.7 Deployment tiers — **Tailscale is one option, never a dependency**

Two web-platform rules decide this, and both are hard:

- **A WebAuthn RP ID must be a registrable domain. An IP address is not one** — the working group
  considered allowing it and [declined](https://github.com/w3c/webauthn/issues/1358). `localhost` is
  the only non-domain exception. So passkeys are impossible on `http://192.168.1.24:8790` *and* on
  `https://192.168.1.24:8790`. HTTPS is not the missing piece; a hostname is.
- **iOS Web Push works only for Home Screen web apps**, over HTTPS, since iOS 16.4
  ([WebKit](https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/)). Safari 18.4 adds
  Declarative Web Push, which drops the service-worker requirement but not the install-first one.
  Android/desktop Chrome and Firefox need HTTPS plus a service worker.

Service workers, PWA install and the Push API all require a **secure context**, which a LAN IP over
plain HTTP is not. That produces a genuine ladder rather than a preference:

| Tier | Origin | Passkeys | Push | Install | Notes |
|---|---|---|---|---|---|
| **0 — just run it** | `http://192.168.1.24:8790` | ✗ (no domain) | ✗ (not secure) | ✗ | Pairing code + bearer token. Honest about being cleartext on the LAN |
| **1 — proxy + cert** | `https://kampr.home.example.com` | ✓ | ✓ | ✓ | **The recommended tier.** NPM with a DNS-01 wildcard gives a real cert on a LAN-only hostname — no port forwarding, no exposure, no Tailscale |
| **2 — public** | same, public DNS | ✓ | ✓ | ✓ | Tier 1 plus reachability. The passkey is now load-bearing |
| **3 — Tailscale** | `https://kampr.tail…ts.net` | ✓ | ✓ | ✓ | Convenience alternative to 1/2, not a prerequisite for either |

Tier 1 is the sweet spot for an NPM user and deserves to be the documented default. What Kampr must
do to make the ladder real:

- **Configurable bind** (`host:port`), loopback by default, explicit opt-in to bind wider.
- **Terminate TLS itself *or* trust a proxy** — an explicit `trust_proxy` setting governing whether
  `X-Forwarded-Proto` / `X-Forwarded-For` are believed. Never inferred.
- **Configurable RP ID / canonical origin.** A passkey registered on one origin does not work on
  another, so the setup flow has to ask which URL is *the* URL, and warn when it changes.
- **Graceful degradation, stated plainly.** On Tier 0, hide the passkey and notification affordances
  and say what a hostname would unlock — do not offer a control that silently cannot work.
- **Tier 0 auth that is real for what it is**: pairing code → device-bound bearer token, LAN bind, a
  persistent "unencrypted" banner, and an expiry that forces a deliberate decision to keep going.

Not recommended, but worth knowing: a self-signed cert on an IP gets you a secure context after the
interstitial (so service workers may work) but still **no passkeys**, because the RP ID rule is about
the hostname, not the transport.

### 3.8 Agent vs. plain terminal — **detectable, and it should pick the default view**

`pane.list` / `session.snapshot` panes carry `agent` (`"claude"`, `"codex"`, …) only when one is
detected; a plain shell pane simply has no `agent` key and reports `agent_status: "unknown"`
**[probed]**. The snapshot additionally precomputes `agents[]` as the subset of panes carrying one.

So the default view is a two-line rule:

| Pane | Default | Conversation tab |
|---|---|---|
| `agent` set **and** a journal adapter exists | **Conversation** | shown |
| `agent` set, no adapter for that harness | Terminal | hidden |
| no `agent` (shell, build, log tail) | Terminal | hidden |

Remembered per pane per device once overridden. Two consequences worth designing for:

- **The common case never shows a 94-column grid on a phone**, because agent panes open in a view
  that reflows natively. That removes most of the pressure on §3.4's lease.
- **A blocked prompt can be answered from Conversation without leasing anything.** The question is in
  the transcript (the harness writes the tool-use request as it asks), while the answer goes back as
  `pane.send_keys {keys:["1"]}`. Reading and writing are independent surfaces — the answer strip needs
  no terminal stream at all.

Stale-session caveat from §1.10 applies: compare `agent_session.agent` against the pane's own `agent`
before trusting a session ref to pick an adapter.

### 3.9 Keyboard mechanics — the part that is genuinely fiddly

The requirement is a real terminal: characters appear as you type them, because the remote echoed
them. `terminal.input` gives that (~27 ms locally, plus link RTT). The hard part is the browser side.

- **The native OSK stays the text source.** The key row is an accessory strip docked above it via
  `visualViewport` — it never replaces the keyboard, so dictation and swipe keep working for free.
- **Per-keystroke capture needs a hidden input, not `keydown`.** Android soft keyboards routinely
  report `keyCode 229` or nothing useful; the workable path is an offscreen `contenteditable` /
  `textarea` read through `beforeinput` / `input` and diffed, which is what browser terminals already
  do.
- **Turn the helpful features off**: `autocapitalize="off" autocorrect="off" autocomplete="off"
  spellcheck="false"`, and handle `compositionstart` / `compositionupdate` / `compositionend` so IME
  and predictive text commit once rather than streaming garbage into the PTY.
- **Echo is remote, and that is correct.** On a fast link it is indistinguishable from local. On a
  slow cellular link it will feel laggy, and the known fix is mosh-style predictive local echo —
  worth listing as an optional later addition, not a launch requirement.
- **Modifier latches** (Ctrl/Alt/Shift/Fn) arm the next tap; long-press exposes alternates. Every key
  the row emits goes out as raw bytes (§3.5), so nothing in the row depends on Herdr's key grammar.

### 3.10 Kampr on the phone itself — **Android plausible, iOS no**

The generalisation worth making first: **Herdr should be a *provider*, not the architecture.** A
kampr node needs three things from a source — enumerate panes, stream a pane, write to a pane. Herdr
supplies them via the socket API plus the terminal streams. Anything else that can supply them is a
provider too, and that abstraction is cheap if it goes in early and expensive if it goes in late.

- **Android: yes, with a local-PTY provider.** Android permits an app to fork/exec and allocate a PTY
  inside its own sandbox — that is exactly how Termux works. So a kampr node on Android can expose a
  shell as a provider without Herdr being involved at all. It runs as the app's uid, so it sees the
  app sandbox and shared storage, not the whole device: useful, not root. Running Herdr itself under
  Termux is a second, jankier option — Herdr ships aarch64 Linux builds, but it is a static-pie glibc
  binary against Termux's bionic userland, so `proot-distro` is the realistic route.
- **iOS: no.** Third-party apps cannot spawn processes, and there is no JIT. Embedded interpreters
  (a-Shell, iSH) exist as whole apps built around that restriction; it is not something Kampr can
  reasonably embed. iOS gets remote panes only, and that is the honest answer.

This is a separate product from the remote-access core and belongs late, but the provider seam
belongs in Phase 1.

### 3.11 Notifications and warm resume

**Notifications** follow the tier ladder in §3.7: Web Push with VAPID on Tier 1+, nothing on Tier 0.
Collie already proves the pattern (`web-push`, VAPID, `push-subscriptions.json`). iOS additionally
requires Add to Home Screen. For Android self-hosters who dislike depending on FCM,
**UnifiedPush** is the idiomatic escape hatch and is worth supporting as an alternative transport.

**Warm resume** is easier than it looks, because of two properties already measured:

- **A reconnect costs exactly one `full: true` frame**, and a full frame is ~2 KB. Herdr coalesces to
  grid state rather than replaying the byte stream — `seq 1 20000` cost 3 frames and 1.9 KB
  **[probed]** — so there is no backlog to catch up on however long you were away.
- Therefore: **render the cached last frame immediately, marked stale, and swap when the fresh full
  frame lands.** No spinner, no blank screen, and the swap is imperceptible because the payload is
  tiny. The visible "load cycle" disappears without any cleverness.

On top of that: cache the herd list so navigation is instant, keep a short-lived resume token so
reconnect skips the auth handshake, and — as suggested — have the **service worker prefetch on push**:
a push for a blocked agent fetches the snapshot and that pane's full frame into cache, so the
notification tap opens onto warm data. Small fetches, well within a push handler's budget.

### 3.12 Herd management — **feature parity with the TUI is achievable**

The design so far assumed Kampr watches and replies. It should also *run* the herd: everything you
would do at the keyboard, minus the keyboard. Probes #46–#50 say the socket carries almost all of it.

| At the desk | Over the socket |
|---|---|
| New workspace | `workspace.create {label, cwd, env, focus}` |
| New tab | `tab.create {workspace_id, label, cwd, env}` |
| Split a pane | `pane.split {target_pane_id, direction: right\|down, ratio, cwd}` |
| Zoom / unzoom | `pane.zoom {pane_id, mode}` |
| Move, swap, reorder | `pane.move`, `pane.swap`, `tab.move`, `workspace.move`, `workspace.move_block` |
| Rename anything | `pane.rename` (null clears), `tab.rename`, `workspace.rename` |
| Close anything | `pane.close`, `tab.close` (closes every pane in it), `workspace.close` |
| Save / restore a layout | `layout.export` → nestable split tree → `layout.apply`; `layout.set_split_ratio` |
| Start an agent | `agent.start {kind, name, pane_id, args}` — 20 kinds on this host, discoverable at runtime via `server.agent_manifests` |
| Git worktree per branch | `worktree.list`, `worktree.create {branch, base, path}`, `worktree.open`, `worktree.remove` |
| Raise a desktop toast | `notification.show {title, body}` |
| Install a harness hook | `integration.install {target}` — 17 targets |

**The one gap: creating a *named session*.** Named sessions are separate Herdr servers and the socket
API has no method for them — the CLI owns it. A node already enumerates them
(`herdr session list --json` gives name, running state and socket path) and can create one headlessly
with `herdr server --session <name>` (#49, and #24 confirms it runs with no client ever attached). So
Kampr supports named sessions; that single action shells out instead of calling a method.

**Split-screening across instances is free, and this is where Kampr beats the TUI.** Each pane is an
independent `observe` stream, and nothing binds a view to one server. So one Kampr window can show a
pane from `comingclean`'s default session next to one from its `agents` session next to one from
`sungrow-pi` — a thing the Herdr TUI cannot do at all, because a TUI client attaches to exactly one
server. Kampr's split is a *client-side* mosaic over the mesh, not a herdr layout.

**This does not contradict §3.4.** "Kampr never resizes a session" means it never reshapes a pane as a
side effect of *viewing* it on a small screen. `pane.split` and `workspace.create` are structural
edits the operator explicitly asked for, exactly as at the desk. The invariant is about
side effects, not about refusing to act.

Two design consequences worth stating:

- **A split changes pane geometry for everyone**, because it changes the herdr layout. That is
  expected and correct for an explicit action, but the UI should say what it will do before doing it —
  the same honesty the zoom control uses, without the modal.
- **`env` on create is a real capability.** `workspace.create` and `tab.create` take an env map, so
  "new Claude session in this worktree with these variables" is one call, not a scripted sequence.

---

## 4. Hard limits — things to stop asking for

| Limit | Evidence |
|---|---|
| No output-change event on the socket API | `pane.output_changed` rejected by the subscription validator; `pane.updated` silent during output **[probed]** |
| `observe --cols` crops, never reflows | 120-char line at 93-col grid observed at 60 cols lost columns 61–93 **[probed]** |
| No input channel that declines geometry | `control` always claims the PTY, even with no size flags **[probed]** — so Kampr uses `observe` + JSON-RPC input, and gives up `terminal.scroll` |
| No scrollback from the frame stream | Frames carry end state only — 29 of 200 lines **[probed]**. The ring comes from `pane.read recent` instead |
| No scrollback ring on alt-screen panes | `max_offset_from_bottom` is 0 **[probed]** — the transcript is their history |
| **Scrollback reads cap at 1000 lines with no way to page further** | `pane.read` has no offset parameter **[probed #51]**. A node that watches continuously accumulates beyond the cap by stitching overlapping reads; history that scrolled past before it started watching is gone |
| **No event when the desk resizes a pane** | Six event types, three verified resizes, zero events **[probed #52]** — geometry change is poll-only |
| `pane.read` drops OSC 8; frames keep it | Verified both ways **[probed]** — the frame path is strictly richer |
| No way to tell whether a desktop client is attached | Not in `ping`, `session.snapshot`, or `herdr status` **[probed]** — now moot, since nothing is ever borrowed |
| Concurrent clients fight over geometry, last writer wins | 100×30 + 60×20 → 18 rows; then 200×50 → 50 rows **[probed]** |
| `pane.read` drops OSC 8 hyperlinks | `\e]8;;url\e\\LINK` → `LINK` **[probed]** |
| `pane.send_keys` cannot name Home/End/PgUp/PgDn/Ins/Del | `invalid_key` on 0.8.2 **[probed]** — irrelevant given raw-byte `send_text` |
| Markdown structure is unrecoverable from any rendered stream | Both read paths are downstream of a renderer |
| Alt-screen panes have no scrollback via `pane.read` | Documented by Collie, consistent with our reads; use `terminal.scroll` or the transcript |
| No cross-host Herdr API | `--remote` is an SSH-hosted TUI client **[docs]** |
| No `session.create` in the socket API | Named sessions are separate servers; creation shells out to the CLI **[probed]** |
| No passkeys on an IP address | WebAuthn RP ID must be a registrable domain — HTTPS does not help |
| No push / service worker / PWA install without a secure context | Plain HTTP on a LAN IP is not one |
| No iOS Web Push outside a Home Screen web app | WebKit, iOS 16.4+ |
| No local shell on iOS | Third-party apps cannot spawn processes |
| No plugin update, no plugin storage API, no sandbox | Plugin v1 **[docs]** |
| `herdr-client.sock` is unusable | bincode, private, unversioned to us **[probed]** |

---

## 5. Recommended architecture for Kampr

```
  phone / tablet / desktop browser  (PWA, Compose Multiplatform, cell-grid renderer)
        │  HTTPS + WSS, passkey-authenticated, device-bound session
        ▼
  ┌─────────────────────────────────────────────────────────┐
  │  kampr node  (hub role)                                 │
  │   • auth: WebAuthn, device tokens, roles, audit         │
  │   • herd model: merged session.snapshot across nodes    │
  │   • stream mux: one observe/control child per pane view │
  │   • transcript service: per-harness journal adapters    │
  └───────────┬──────────────────────────────┬──────────────┘
              │ mesh link (mTLS/token)       │ local
              ▼                              ▼
      kampr node (peer host)          herdr server(s) on this host
              │                              │  JSON socket  +  terminal streams
              ▼                              ▼
      herdr server(s)                 panes / agents / workspaces
```

Deliberate inversions of Collie:

| Collie | Kampr |
|---|---|
| Poll `session.snapshot`; browser polls too | Poll snapshot for *structure*; **stream frames** for *content*; push to the browser over WS |
| No terminal emulator, `StyledLine[]` | **VT emulation in the node**, cell grid to the client (§5.5) |
| Explicit Send button, no live keys | **Live keys** by default, with a Send-a-block affordance kept for dictation |
| Front door *is* the auth | **Auth is in the bridge**; the front door is just TLS |
| One host | **Mesh of nodes**, hub role |
| Mirror wraps text | **Two views**: truthful terminal + transcript-derived markdown |
| systemd only; Herdr never nudges it | systemd/launchd **plus** a `[[startup]]` hook that re-nudges |

What to keep from Collie, without argument: the `herdr-client` adapter seam (one module knows method
names), the audit log, the destructive-command confirm, the PWA build-stamp cache-bust, the
`bridge/journal/` registry shape, and the habit of writing down every probe.


### 5.5 Stack, and the one architectural call it forces

**Server: Rust + axum.** Versions checked against crates.io on 2026-08-20:

| Crate | Version | For |
|---|---|---|
| `axum` | 0.8.9 | HTTP + WebSocket |
| `tokio` | 1.53.1 | runtime, `tokio::process` for the observe children |
| `tower-http` | 0.7.0 | headers and tracing |
| `rustls` | 0.23.43 | own-TLS mode |
| `webauthn-rs` | 0.5.5 | passkeys (Tier 1+) |
| `vte` | 0.15.0 | VT parsing — `crates/kampr-term` is built on it and verified against Herdr's own grid |
| `sqlx` | 0.9.0 | devices, tokens, per-pane prefs |
| `web-push` | 0.11.0 | VAPID push — *not yet added; Phase 8* |
| `quinn` | 0.11.11 | QUIC for mesh links — *not yet added; Phase 4* |


**Clients: Kotlin Multiplatform + Compose Multiplatform.** Kotlin **2.4.10**, CMP **1.11.1** (1.12.0-rc01
in flight), Ktor client **3.5.2** — checked against Maven Central metadata the same day. One codebase
for Android, iOS, wasm and desktop.

**The call this forces: emulate the terminal on the server, not in the client.**

Herdr hands us ANSI *diff* frames. Applying a diff requires full emulator state, so **something** must
run a VT emulator. The options are a Kotlin emulator in `commonMain`, or a Rust one in the node. Rust
wins, and not narrowly:

- **One emulator per pane, not per viewer.** Three devices watching one pane share a single grid.
- **Rust has `vte`, and the required subset is small.** Herdr's serialiser emits absolute cursor
  addressing, SGR, erase and the sync/hyperlink markers — no scroll regions, no relative motion.
  `crates/kampr-term` implements it in ~250 lines and matches Herdr's own grid exactly. A Kotlin
  equivalent would be the same work in the language with the worse tooling for it.
- **The client becomes a renderer.** Compose draws a cell grid — no ANSI parsing, no escape sequences,
  no wasm perf question about parser throughput.
- **Structure becomes available**: selection, copy, find-in-screen, OSC 8 hyperlinks and tap targets are
  server-side features over a cell model rather than three client reimplementations.
- **The wire protocol becomes ours** — versioned and stable — instead of Herdr's frame format leaking
  all the way to a phone.
- **Zoom and pan are trivial on a cell grid**, and a soft-wrap mode stays available for free if that
  preference ever changes.

Collie's ADR 0008 refuses a cell-grid protocol, on the grounds that it reopens pan-vs-wrap. That
objection is spent here: pan-vs-wrap has been *decided*, in favour of pan.

**Build shape.** Gradle builds the CMP wasm bundle → the Rust crate embeds it (`rust-embed`) → one
binary serves the API and the web client with no runtime toolchain. Android ships an APK; iOS ships
the same Compose code in a native shell.

**One iOS wrinkle worth deciding early.** A *native* CMP iOS app cannot use Web Push — it needs APNs,
which needs an Apple developer account and a push relay you host. The PWA route avoids APNs entirely
but requires Add to Home Screen (§3.7). Pick one before Phase 8, not during it.

### 5.6 Install

Three routes, and none is a prerequisite for another.

**As a Herdr plugin** (the turnkey path):

```bash
herdr plugin install <owner>/kampr
```

The manifest's `[[build]]` step downloads the matching prebuilt binary rather than compiling — no Rust
toolchain on the user's machine. Then `[[actions]]` give start / stop / status / url / update /
uninstall, `[[panes]]` with `placement = "popup"` opens the setup ladder in a Herdr popup, and
`[[startup]]` re-nudges the service after a Herdr restart or live handoff. Remember plugin v1 has no
`plugin update` and a reinstall restarts nothing (§1.9), so `update` is our action to own.

**Standalone**, for people not using the plugin surface:

```bash
curl -fsSL https://kampr.dev/install.sh | sh
kampr init            # config, node keypair, URL + pairing code, QR
kampr service install # systemd --user unit (launchd on macOS)
```

**From source**, `cargo install --path .` plus a Gradle build for the web bundle.

First run works immediately at Tier 0 — LAN bind, pairing code, no certificate, no account. Everything
above that is an optional rung on §3.7's ladder, offered from the setup screen and never demanded.

---

## 6. Open questions to resolve before/while building

1. **Does a live OSC 8 hyperlink survive a `terminal.frame`?** The serialiser emits resets, which is
   suggestive but not proof. Cheap to probe; decides whether tappable links come free.
2. **What is the frame stream's behaviour when the pane closes mid-stream** — `terminal.closed` with
   which reason, and how fast?
3. **Does `observe` accept an agent-name target** as well as a pane id? `<TARGET>` is untyped in help.
4. **Cost of one child process per viewed pane.** With 20 panes and 3 devices, is it 60 processes?
   Almost certainly Kampr should mux: one stream per (pane × geometry), shared by viewers at the same
   size, refcounted.
5. **Does `terminal.scroll` reach real scrollback on an alt-screen pane, or only on shell panes?**
   Decides whether the terminal view can scroll back at all, or whether scrollback is transcript-only.
6. **Version floor.** `observe`/`control` land in 0.7.2; `--cols/--rows`, and the coalescing behaviour
   we measured, need a floor pinned by probe. `min_herdr_version` in the manifest must reflect it.
7. **Public exposure story.** Even with passkeys, is the recommendation "Tailscale by default, public
   opt-in", or genuinely public-first? Changes the wizard's shape and the threat model docs.
8. ~~**Does `pane.read source:"recent"` scroll a plain shell pane?**~~ **Answered: no** — 0.002 s,
   viewport unmoved, full colour. Still open for a shell pane that *has* a detected agent; the
   interlock assumes the worst there.
9. **Is a *pending* tool request flushed to the transcript before it is approved?** In a finished
   session all 300 `tool_use` blocks had results **[probed]**, which proves nothing about ordering.
   Decides whether the Conversation view's answer strip sources its question from the transcript or
   from `pane.read visible`. The feature works either way; only the source changes.
10. ~~**Does `observe` at the pane's exact native size render identically?**~~ **Answered: yes.**
    `crates/kampr-spike` reconstructs the grid from frames alone and matches `pane.read visible`
    **30/30 rows**, with the cursor on the right cell and a hyperlink recovered that `pane.read`
    would have dropped.
11. **Compose Multiplatform wasm: can it draw a 94×40 cell grid at 60 fps on a mid-range phone?**
    Needs a spike before the client architecture is committed.
12. **iOS push: native APNs or PWA Web Push?** Decides whether an Apple developer account and a push
    relay are on the critical path.
