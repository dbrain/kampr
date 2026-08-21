# Architecture — Kampr (a Herdr bridge that renders a real terminal)

> **Why Kampr is shaped the way it is.** The deployment model, the path a frame takes from a Herdr
> pane to a phone, and the handful of decisions that were genuinely contested — the reasoning the
> code cannot state itself. Nearly every load-bearing decision here rests on a measurement rather
> than an opinion, so claims about Herdr carry a probe number pointing at
> [`docs/03-probe-log.md`](./docs/03-probe-log.md).
>
> For how to run it see [`README.md`](./README.md). For why a road *wasn't* taken, the ADRs in
> [`docs/adr/`](./docs/adr/README.md). For what Kampr does and does not defend,
> [`docs/08-threat-model.md`](./docs/08-threat-model.md). For what is broken or unbuilt right now,
> [`docs/06-audit.md`](./docs/06-audit.md) — that file is the live record and this one does not
> duplicate it.

## 1. The problem

[Herdr](https://herdr.dev) runs a herd of coding agents in terminal panes at a desk. The agents keep
working when you leave the desk; the terminal does not follow you. The existing route away from it is
SSH from a phone into the machine and run the TUI, which is a bad keyboard on a screen that is the
wrong shape for a 94-column grid.

The nearest prior art is [Collie](https://github.com/AltanS/collie), a Herdr web bridge that solves a
narrower problem very well: triage. A NEEDS-YOU list, a push notification, a tap, a structured prompt
block, an explicit Send. The pane text is context below the fold, rendered as wrapped rows. That is
a coherent product and Collie's architecture documents are the reason it is maintainable; they are
worth reading before this one.

Kampr wants something Collie deliberately does not: **the actual terminal**, live, on a phone,
across several machines, behind authentication good enough to expose — without reshaping the session
you left running at your desk.

Those two products collide architecturally rather than incidentally, which is why Kampr is a separate
plugin rather than a fork. Where they disagree, the disagreements are written down as ADRs and
Collie's position is stated in its own words first.

## 2. What Kampr is

A **kampr node**: a long-lived local process, one per host, that is the only thing which touches a
Herdr socket or spawns a Herdr terminal stream. It serves a WebSocket protocol and a bundled web
client, and it holds the authentication.

(One process, but *"node"* in the wire model means a **herdr server**, not a machine: a named session
is a separate server with its own socket (#49), so one process presents each of its host's sessions
as its own node — the configured one under the node's own id, the rest suffixed. Ids are opaque and
must be matched exactly, never by prefix.)

```
   phone / tablet / desktop browser / Android app
        │  one WebSocket at /ws, JSON text frames, device-bound token
        │  (the wasm bundle is baked into the binary; no CDN, strict CSP)
        ▼
   ┌──────────────────────────────────────────────────────────────────┐
   │  kampr node                                                      │
   │   • auth      tiers, devices, tokens, passkeys, roles, audit     │
   │   • herd      merged snapshot: nodes, sessions, panes, agents    │
   │   • registry  ONE VT emulator per pane, refcounted by watchers   │
   │   • history   a stitched scrollback ring per shell pane          │
   │   • journals  per-harness transcript adapters                    │
   └───────┬──────────────────────────────────────────┬───────────────┘
           │ mesh link (ed25519, peer dials hub)      │ local
           │        — see §7                          │
           ▼                                          ▼
   kampr node on another host              herdr server(s) on this host
           │                                    │
           ▼                            ┌───────┴────────┐
   herdr server(s)                      │ socket JSON-RPC│  structure
                                        │ terminal stream│  content
                                        └───────┬────────┘
                                                ▼
                                     panes · agents · workspaces
```

Four properties are worth stating up front because everything else follows from them.

**Kampr streams frames and emulates the terminal in the node.** Not because rendering is prettier
that way, but because Herdr's frame stream carries information its read API destroys — the cursor,
and OSC 8 hyperlinks — and because applying a diff frame requires emulator state somewhere.
[ADR 0001](./docs/adr/0001-the-node-runs-a-vt-emulator.md).

**Kampr cannot resize a pane.** Not "does not" — *cannot*. The code path does not exist, because the
only Herdr API that could resize also seizes the PTY from the person at the desk with no way to
decline (#17). [ADR 0002](./docs/adr/0002-kampr-never-resizes-a-pane.md).

**Clients receive a cell grid and parse no escape sequences.** Herdr's frame format stops at the
node. [ADR 0003](./docs/adr/0003-the-client-contract-is-a-cell-grid.md).

**Agent panes default to a conversation view built from the harness's own transcript**, because a
markdown table cannot be recovered from a rendered grid, and because that view reflows natively so
the common case never shows a 94-column grid on a phone.
[ADR 0005](./docs/adr/0005-structure-comes-from-the-transcript.md).

## 3. Deployment model

### The node is a supervised service; the plugin is a launcher

A Herdr plugin can host a process in a pane. That is exactly the wrong home for this one: a node
that only lives as long as a terminal pane dies while you are away from the desk, which is the
entire scenario. So the node runs as a `systemd --user` service (a launchd agent on macOS) and
outlives Herdr, and the plugin manifest is a launcher around it.

Outliving Herdr is not the same as outliving a reboot, and the difference is one command. A
`systemd --user` manager lives inside the user's login session: it is torn down when the last
session ends and it is not started at boot unless that user lingers. So `kampr service install`
runs `loginctl enable-linger` and, when it cannot, prints the command as a required next step
rather than a suggestion — and `kampr doctor` fails on a unit installed without it. The launchd
half has the same shape for a different reason: a `gui/$UID` agent needs someone to log in at the
screen, so a headless Mac needs a LaunchDaemon instead, which `doctor` says rather than implying.

Herdr's plugin host makes three of these choices for us and they are worth knowing:

- **`[[startup]]` hooks are one-shot, not supervised.** They fire after session restore and again
  after a live handoff, which makes them a perfectly good *nudge* — better than nothing, and not a
  substitute for a service unit.
- **Plugin v1 has no `plugin update`.** Herdr's refresh is a reinstall, which replaces the checkout
  and restarts nothing we started. So the `update` action is ours to own.
- **`[[build]]` downloads a prebuilt binary rather than compiling.** Requiring a Rust toolchain on a
  user's machine in order to read a terminal on their phone would be absurd.

`[[panes]]` with `placement = "popup"` is the setup vehicle: session-modal, receives Escape, closes
when its command exits.

### One binary, no runtime toolchain

Gradle builds the Compose Multiplatform wasm bundle; a Sync task stages it into the crate; `rust-embed`
bakes it into the binary. One artefact serves the API and the web client, with a strict CSP and no
external origins — everything a client needs is bundled.

This was a *silent* gap for most of the project's life and is the best cautionary tale in it: nothing
ever put the bundle where the node looked. No Gradle task, no script, no CI step. `dist/` held a
`.gitkeep` explaining that something ought to copy files there, so **every node ever built served the
placeholder page**, and nobody had seen the client work against the node until the day it was wired
up (`d11260f`). Three wire bugs were hiding behind that, all of the same shape: both sides agreed with
each other and neither agreed with the other's bytes.

### The tier ladder is dictated by a browser rule, not by taste

The first run works immediately: the node starts, prints a URL and a pairing code, and serves. Setup
is then a screen of labelled upgrades rather than a six-step gate.

What makes it a *ladder* rather than a preference is a web platform rule with no workaround:

> **A WebAuthn RP ID must be a registrable domain. An IP address is not one.** HTTPS does not help —
> a self-signed certificate on `https://192.168.1.24:8790` gets you a secure context and still no
> passkeys, because the rule is about the name, not the transport.

Service workers, PWA install and the Push API separately require a secure context, which plain HTTP
on a LAN IP is not. So what an origin *is* decides what the product can offer:

| Rung | Origin | Credential | Push / install |
| --- | --- | --- | --- |
| Just run it | `http://192.168.1.24:8790` | pairing code → device token, 30-day expiry | none |
| **Hostname + certificate** | `https://kampr.home.example.com` | **passkey**, no expiry | available |
| Public, or over Tailscale | same, differently reachable | passkey | available |

The middle rung is the recommended one and deserves to be the documented default: a reverse proxy
with a DNS-01 wildcard gives a real certificate on a LAN-only hostname, with nothing exposed and no
Tailscale.

Two implementation rules follow, and both are load-bearing:

- **The tier is derived from the configured origin, never from the request.** A request cannot tell
  the node what it is. The same rule governs the same-origin allowlist, which is built from the bind
  address rather than reflected from the request's `Host` — a reflected allowlist satisfies itself
  under DNS rebinding.
- **What cannot work is absent, not offered and failing.** `hello.security` tells the client what
  this origin supports, and `unlocks` names what a hostname would buy. The client builds its ladder
  from that message and never by parsing the URL, so on an IP the passkey button does not exist
  rather than existing and failing at the last step.

One honest correction to the table above: the wire's `security.tier` field only ever carries **0 or
1**, because the only thing detectable from an origin is whether passkeys are possible. "Public" and
"over Tailscale" are deployment postures, not detectable states.
[`docs/04-wire-protocol.md`](./docs/04-wire-protocol.md) still documents `tier` as four-valued and
should be corrected. [ADR 0006](./docs/adr/0006-auth-is-in-the-node.md).

## 4. The data path, Herdr to a phone

### 4.1 Two planes, and only one of them carries content

Herdr is not one API. Kampr uses two of its three planes and refuses the fourth:

| Plane | What Kampr does with it |
| --- | --- |
| **Socket JSON-RPC** (`~/.config/herdr/herdr.sock`) | Structure only — the herd model, input, one-shot reads, `manage` operations. Versioned, schema-published. |
| **Terminal streams** (`herdr terminal session observe`, a CLI child) | Content — every pixel a client ever sees. Documented for third-party bridges; no protocol version of its own. |
| **Plugin host** | Installation, actions, the setup popup, the startup nudge. |
| `herdr-client.sock` | **Never.** It is bincode-framed, private and unversioned to us — and it is exactly the structured grid a web client would want, which is precisely why it will break on every Herdr release. On the roadmap's cut list. |

The split is forced rather than chosen. **There is no output-change event on the socket API**:
`pane.output_changed` exists as an event kind and the subscription validator rejects it, and
`pane.updated` stayed silent through three seconds of output (#4, #5). A bridge built on the socket
alone has no option but to poll, which is what Collie does and does well. The frame stream is the
only live content path there is.

Structure still comes from polling `session.snapshot` every 3 s, poked by an event subscription with
a 60 ms settle. Most of that subscription is a fixed list of topology events. One is not:
`pane.agent_status_changed` is rejected by Herdr unless it names a `pane_id`, and **one invalid entry
rejects the whole `events.subscribe` call** (#54) — so it has to be subscribed per pane and
re-subscribed whenever the pane set changes, and a mistake there silently costs every other
subscription as well.

### 4.2 Geometry is the number nobody reports

Before a stream can be opened, the node has to know how wide the pane is, and this turns out to be
the hardest small problem in the system.

`observe --cols` **crops; it does not reflow** (#15). A 120-character line on a 93-column grid
observed at 60 columns loses columns 61–93 entirely — they are not wrapped to the next row, they are
gone. So the observed geometry has to be the pane's actual geometry, and being wrong is not a
cosmetic error.

`observe` with no size flags defaults to 120×40 rather than to the pane's real size (#16), so the
flags are mandatory.

The pane's geometry is in `session.snapshot`'s `layouts[].panes[].rect` (#33) — and **no event fires
when the desk resizes a pane.** Six event types, three verified geometry changes, zero events (#52).
`layout.updated` covers structural change only. So native geometry is poll-only and the 3 s poll is
load-bearing rather than a backstop.

Worse, in a headless session — the configuration both the plugin and the service produce — **the PTY
does not follow the layout rect at all** (#68). A pane whose rect said 47 columns had a 93-column
PTY, so observing at the rect cropped every row in half; and observing *above* the PTY width merely
pads, while the `width` a frame reports just echoes what was requested and so carries no information
at all (#87). Nothing in the socket API reports a pane's real column count: `pane.get` carries
`viewport_rows` and no columns.

So the node **infers** it, from the one thing that does render at the true width: `pane.read` (#84).
`recent` returns physical rows already wrapped at the PTY's own width, and `recent_unwrapped` returns
the logical lines they came from. A logical line longer than the widest physical row proves a soft
wrap happened, and a soft wrap happens at exactly the PTY width — so that widest row *is* the width,
whatever the rect claims (#85: `recent` gave rows of `[5, 68, 93, 93, 93, 93, 28, 34]` where
`recent_unwrapped` gave `[5, 68, 400, 34]`, and 93 was the answer). Without a wrap there is no proof,
only a lower bound, which is why it combines with the rect by `max` and is re-measured on a poll of
its own. This is the one place in the system that reasons from evidence rather than from a reported
number, and it is worth understanding before touching it.

### 4.3 Frames in, cell grid out

`herdr terminal session observe <pane> --cols N --rows M` is spawned as a child process, one per
watched pane, with `kill_on_drop`. It emits NDJSON: base64 ANSI frames with a sequence number, a
size, and a `full` flag. Only the first frame of a stream is `full` (#53); the rest are
cursor-addressed partial repaints, and every frame is wrapped in synchronised-output markers and ends
with an absolute cursor address (#12).

`crates/kampr-term` applies them. It is deliberately small — about 450 lines over `vte` — because
Herdr's serialiser emits a small subset: absolute cursor addressing, SGR, erase, and the
sync/hyperlink markers. There are no scroll regions, no save/restore, no alt-screen handling, because
nothing upstream produces them. The result is a cell grid with per-row dirty tracking and an interned
link table. `crates/kampr-spike` reconstructs a pane from frames alone and diffs it against Herdr's
own `pane.read visible`; it matched **30/30 rows** with the cursor on the right cell and recovered a
hyperlink that `pane.read` drops (#41, #36, #37). It is the pipeline's canary and it is expected to
print `PERFECT MATCH`.

The registry holds **one emulator per pane**, refcounted by watchers rather than by a counter — the
entry is held weakly and the last `Arc` dropping tears down the stream, the history poller and the
child process together. Two viewers of one pane share one emulator and one child; a new watcher waits
up to two seconds for a real frame rather than being handed a placeholder grid.

What a client sees on each kind of interruption is the interesting part, and in every case it is a
single message:

| Event | Client sees |
| --- | --- |
| herdr dies under a live watcher | `error{herdr_unavailable}` on the connection; the pane stays in the herd model, because a herdr restart keeps its workspaces and panes (#70) |
| herdr restarts at the same geometry | Nothing until the next `full` frame, which publishes exactly one `grid.reset`. No blank flash |
| the desk resizes the pane | One `grid.reset` at the new size, every row |
| a watcher falls behind the broadcast | One `grid.reset`, never a queue of patches |
| the client's socket congests | Its `grid.*` queue is purged and one `grid.reset` is sent |

A `grid.reset` is a few kilobytes and Herdr coalesces bursts to end state — `seq 1 20000` cost **3
frames and 1.9 KB** (#23) — so there is never a backlog to catch up on. That is what lets a client
render its cached grid immediately, marked stale, and swap when the reset lands. There is no spinner
anywhere in the product, and that is a consequence of a measurement rather than a style choice.

The wire encoding interns styles into a per-connection append-only table and run-length encodes rows.
Measured against per-cell JSON: **61×** smaller at 124×50 with 49 distinct pens, **44×** at 74×30 with
light colour.

**One rule about backpressure is not obvious and is load-bearing:** only `grid.reset` and
`grid.patch` may be purged. History is append-only and a reset carries the viewport and nothing above
it, so a purged `scrollback` is a permanent unsignalled hole; and a purged `styles` entry orphans
runs that survive it. This shipped wrong once, with a catch-all classification that made scrollback
purgeable — effectively a row cap of exactly the kind the design had just ruled out.

### 4.4 Scrollback is the other half of the surface, and it is the compromised half

**Frames cannot supply history.** `seq 1 200` on a 30-row pane put 29 distinct lines across the
*entire* frame stream — the final viewport. Lines 1–171 were never transmitted (#25). A frame-fed
emulator cannot rebuild scrollback however long it has been watching. And `terminal.scroll` is a
control-mode command, which Kampr does not use (§2, ADR 0002).

So history comes from `pane.read recent format=ansi`, run through the same emulator so styling
matches, behind an interlock: read only when the ring is non-empty **and** the pane has no detected
agent. The second half is Collie's documented hazard — on an idle recognised-agent pane, a deep read
harvests through the agent's own mouse-scroll interface and visibly moves the operator's screen. The
interlock assumes the worst about that case rather than testing it, which is the right default and
is now looser than the evidence requires: probe #86 found a read at exactly `viewport_rows` safe on
a detected-agent pane, leaving the viewport unmoved. Narrowing the interlock to depth rather than to
the presence of an agent is a change worth making on that evidence.

Agent panes lose nothing by this. They are alt-screen, so `max_offset_from_bottom` is 0 and no ring
exists to miss (#30), and their history is the conversation.

Herdr caps a read at **1000 lines and takes no offset parameter** (#51), so deep history cannot be
paged to at all. A node that *watches* is better placed: successive reads overlap, and the overlap is
proof of adjacency, so the node stitches them into a ring that grows past the cap — proven live at
1553 rows with every marker accounted for.

**When two reads share no overlap, the node discards what it held.** The two stretches are not
adjacent and nothing can prove what sits between them; splicing them would make `from_top` and
`total_rows` fiction, and a client would render two unrelated stretches as one document with no way
to know. So the ring restarts, `from_top` advances by the number of rows dropped so absolute indices
stay true, and `capped: true` says what happened. A width change restarts the ring for a different
reason — every stored row was wrapped at the old width — and the log distinguishes them.

Polling is adaptive to keep that rare: `clamp(400 rows / measured rows-per-second, 100 ms, 2 s)`, with
the rate taken from rows actually appended by the previous stitch rather than from frame content
(Herdr coalesces, so counting newlines in frames under-counts by three orders of magnitude). An idle
pane is not polled at all — the poller waits on a notify fired per frame with a 30 s backstop.

It is not enough, and the honest statement is that it cannot be. A sustained thousand rows per second
now survives; a four-thousand-row instant burst still gaps, which is arithmetic against a 1000-row
cap. Probe #71 saw it in ordinary use: `seq 1 40000` in a watched pane restarted the ring. Preserving
history across a gap needs a per-segment `from_top` or a gap sentinel — a two-field wire addition,
specified but deliberately not in v1, and the most defensible open item in the protocol.
[ADR 0004](./docs/adr/0004-scrollback-is-stitched-and-a-gap-discards.md).

The node's ring bound is **20 000 rows**, roughly 4 MB of raw ANSI. It is a memory limit, not a
display one, it is configurable, and **clients must not impose a cap of their own.**

### 4.5 Conversation is a third source, not a rendering of the second

A markdown table that reached a terminal has become box-drawing characters in cells. Both of Herdr's
content paths are downstream of a renderer, so the structure is already gone and no amount of better
emulation recovers it. Collie's ADR 0008 puts it best: *a TUI paints cells; it does not paint
structure.*

But the harness wrote the original markdown to disk. `~/.claude/projects/<slug>/<uuid>.jsonl` parses
to literal markdown (#39); Codex writes `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (#45). So an
agent pane has two views over one session — the terminal, where you type, and the conversation, which
is read-optimised, phone-shaped, and scrolls through the whole session rather than one viewport.
Adapters are keyed on Herdr's own `agent` string, and registered only if their root directory exists,
so a pane can never claim a conversation the node cannot serve.

Two probes shaped this more than any argument. **Claude does not write a pending tool request to its
transcript before you approve it** — a session held at a permission prompt left the JSONL frozen for
4 m 20 s and then jumped, carrying both the request and its result together (#42). **Codex does**
(#43). Since Claude is the harness targeted first and it will not tell us the question until it has
been answered, `pending` is sourced from `pane.read visible` and the wire says `source: "screen"`.
The shape is identical either way and **clients must not care which**.

The node decides whether a submit key follows an answer, per harness. Probe #72 confirmed live that
Claude acts on the bare digit for both its trust prompt and a real `Bash` permission dialog —
including the one whose footer reads "Enter to confirm".

[ADR 0005](./docs/adr/0005-structure-comes-from-the-transcript.md).

### 4.6 Input is a separate, stateless surface

Reading and writing are independent. Input goes over one-shot JSON-RPC — `pane.send_text` or
`pane.send_keys` — with no ownership and no session state, which is what makes it possible to type
into a pane without claiming its geometry.

`pane.send_keys` has a grammar that rejects `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete`
and `BackTab` on 0.8.2 (#8). That would be a problem except that **`pane.send_text` writes raw
bytes** — verified against `cat -v`, which echoed `^[`, `^[[5~`, `^[[H`, `^A` and UTF-8 intact (#9).
So every key Herdr's validator refuses goes out as its escape sequence, and a test asserts that no
key-row layout depends on `send_keys`. There is no key Kampr cannot deliver.

Two consequences of raw bytes are easy to miss:

- **Paste must supply its own bracketed-paste framing.** `send_text` writes bytes with no framing, so
  an unframed multi-line paste executes line by line in a shell. The web client does this in its
  input shim; the helper written for the other two platforms has no callers, so an Android or desktop
  paste currently goes out unframed. The rule is right and one implementation of it is missing.
- **`input.b64` is a convenience for control characters, not a raw-byte escape hatch.**
  `pane.send_text` takes a JSON string, so bytes that are not valid UTF-8 have no representation on
  the wire to Herdr at all; invalid UTF-8 is rejected rather than mangled. Nothing the key row needs
  is lost, because every escape sequence it emits is UTF-8-safe.

Echo is remote, and that is correct: characters appear because the pane echoed them, at a measured
p50 of 27 ms locally (#22). On a slow cellular link it will feel laggy, and the known fix is
mosh-style predictive local echo — worth listing, not worth launching with.

### 4.7 Notifications are the reason a phone client exists

Everything above is about looking at a pane. The reason to have the product at all is being *told*
that one needs you, without looking.

Push is gated by the ladder rather than by preference: the Push API needs a secure context, so it is
unavailable at the bottom rung and `hello.security` says so. Above it, the node holds one VAPID
P-256 keypair generated at `kampr init` and kept at 0600 beside the database.

Three design points are worth carrying forward:

- **A push subscription lives in the auth database, not beside the push code.** It is a standing
  invitation to wake a phone and is exactly as sensitive as the device token next to it, so it
  belongs under the same revocation — the target list is a join against live devices rather than a
  cleanup job somebody has to remember.
- **A UnifiedPush endpoint is a Web Push endpoint.** UnifiedPush 3.0 carries RFC 8291 encryption and
  VAPID, so the same delivery code serves it; only who hands over the endpoint differs. That is why
  the transport is a label and never a branch, and it is what lets an Android self-hoster avoid FCM
  without a second implementation.
- **Blocks are batched into a short collection window**, so three agents finishing a batch of edits
  arrive as one notification rather than three racing ones. The window starts at the first block and
  does not extend, so a steady trickle still notifies once per window rather than never.

**The notification body carries the agent's actual question**, which is the known gap in the prior
art — identifying which agent needs you and making you open the app to find out what it wants is only
half the job. It is also a deliberate disclosure trade, because a body is a notification's whole
content on a locked screen. See the threat model, §7.9.

## 5. The client

One Kotlin Multiplatform codebase, Compose Multiplatform, targeting Android, JVM desktop and wasm.
Six modules: `shared` (wire codec, store, tokens, navigation, and the `PaneSurfaces` seam), `terminal`
and `conversation` (which decorate that seam and can be composed independently), and three thin
composition roots.

### 5.1 The terminal fills the viewport, and paint is not content

Blank space below the last row is a bug, not a layout. Three rules make that true, and they are the
kind that get quietly broken by the next layout change:

**Scrollback and the live grid are one continuous surface**, addressed by a single index — history
runs `[0, historyRows)` and the live viewport follows it, pinned to the bottom. There is no divider
and no separate scrollback mode, which is what lets the space above a short grid carry history rather
than nothing. Only visible rows are ever read out of it, so two thousand rows of history above a live
grid costs 0.7 ms.

**Default zoom fills at least one axis**: `max(fit-width, fit-height)`, never `min`, because fitting
inside both axes is exactly what letterboxes. A pane with no ring has nothing above it to fill with,
so fill-height wins and it pans horizontally instead — and those panes default to the conversation
view anyway.

**Paint and content are two different rectangles**, and conflating them produces either a letterbox
or a permanently hidden row. The terminal *paints* the whole viewport, so rows run under the header
and the key row and nothing is ever blank. The *scrollable content* is inset by that chrome, so the
pinned last row settles clear of it. Same semantics as safe-area insets, and the reason chrome can
float over the terminal while staying opaque. **Fill is computed against the paint rectangle** — inset
it first and the insets reintroduce the letterbox they exist to prevent. There is a test whose whole
purpose is to say so.

The same mistake has been made twice in different places, which is why it is written down: the key
row once laid itself out against the window height and the Android navigation-bar inset, leaving a
static gap the height of the nav bar between the row and the keyboard. It is an accessory to the
keyboard and must butt against it, which means tracking `visualViewport` live through the show/hide
animation on the web and the *animated* IME inset on Android.

### 5.2 Two render modes, and a negative result worth keeping

A cell grid holds 60 fps at 74×30 on real WebGL2 (0.6 ms draw) and on Android (1.77 ms), against a
16.67 ms budget, with zero dropped frames of 300; 200×50 holds too (#56). **Fill is free and text
shaping is the entire cost** (#58) — which makes caching shaped run layouts a requirement rather than
an optimisation. Shaping every frame is ~53 % of budget on wasm and ~85 % on Android. The measured
cache hit rate is 99.2 %, because terminal output repeats run strings far more than intuition
suggests.

Two things follow. **Pinch must layer-scale during the gesture and re-shape only on settle**, because
re-shaping at intermediate zooms collapses the hit rate to ~51 % (#60). And the cache has a worst
case: under churn, cached runs fall to 30 fps with 46–57 % dropped. So the renderer has a second mode
— per-glyph `drawText`, which holds 60 fps there (#62) — and switches on measured cache health with
hysteresis, dropping below 0.70 and rising above 0.90 over an eight-frame window. In per-glyph mode
nothing populates the run cache, so the recovery signal is frame-to-frame overlap of the run-key set
instead: a lower bound on what the cache would have achieved, which is what makes coming back safe.

**A hand-rolled glyph atlas was built and is a genuine negative result** (#61). It was the *fastest
mode measured anywhere* on Android at 2.53 ms — and 2.2 fps on JVM desktop, and a Skia abort every
single frame on wasm. Rendering was visually correct in all three, so it is the API path and not the
idea: Skia already keeps its own GPU glyph atlas, and per-glyph `drawText` is how common code reaches
it. Do not build a second one.
[ADR 0008](./docs/adr/0008-two-render-modes-not-a-glyph-atlas.md).

### 5.3 Selection, links and the key row

Selection is long-press and drag with handles and a floating Copy, **linear by default** because that
is what selecting a paragraph does everywhere else. What gets copied is the *logical* text — trailing
padding stripped, soft-wrapped rows rejoined — since a path copied with a newline through the middle
of it is worse than not copying at all.

Links cost almost nothing because the cell model already carries a link id into the pane's table, so
an OSC 8 hyperlink is a real harness-declared URI. Bare URLs are detected too, but **detected is not
declared**: pane output is attacker-influenceable, so detection is a strict scheme match rather than
"anything with a dot in it", it runs over logical lines so a URL wrapped at the grid edge is not
missed, and **nothing ever auto-navigates** — a target is shown with an explicit Open.

The key row puts arrows in an inverted T with navigation grouped to the right of a fixed separator
track, because every physical keyboard puts up directly above down with left and right flanking, and
an L-shape is what makes a thumb look down. Landscape keeps the same cluster, widens the left group
and gains a symbols row, since horizontal space is what landscape has to spend. The reasoning behind
a tighter landscape row is that the 44 dp touch-target guideline assumes a one-handed reach, and a
landscape key row is a two-thumb posture at the screen edges where 44 dp costs a quarter of the
screen. **What ships is not what is written down**, though: the wire protocol document specifies
36 dp in landscape, and the caps keep a 44 dp minimum height in both orientations and take the
saving out of vertical padding instead. One of the two should move. Measured on a 411 dp portrait
emulator a cap is 42 × 44 dp — 44 tall as written, and 42 wide because eight caps and their gaps do
not fit at 44 each across 411 dp, which is a fact about the screen rather than a decision anyone
made.

### 5.4 One place a control can be made, and therefore one place it can be named

Nothing in the client uses `Modifier.clickable` directly. Every interactive control goes through
`Modifier.action` (or `Modifier.gestureAction`, where a raw gesture detector is doing the work),
which is a single seam carrying four things a control needs and none of which survives being
remembered per call site: a **name for the action rather than the glyph** — "Zoom, currently 1.6×",
never "magnifier" — a role, a focus ring, and a semantics click. That last one is not cosmetic: a
screen reader's activation dispatches the *semantic* click, so the key row's caps were reachable,
unnamed and impossible to press at the same time until they had one (probe #92).

Status is a shape as well as a colour — a square, a disc, a bar and a ring at 7 dp — because a
coloured dot is one channel and roughly one reader in twelve does not have it. Anything that
appears without the operator acting is a live region: the "Needs you" list, a pending answer, the
destructive-command sheet, a stale pane, a node dropping off the mesh. Sheets clear the semantics
of the screen behind them rather than trapping focus, which buys the same thing without turning a
sheet into a platform dialog, and Escape closes one.

`prefers-reduced-motion` is read per platform — Android's animator scales, the browser's media
query, GNOME's `Gtk/EnableAnimations` over XSETTINGS — and gates the cursor blink, the momentum-pan
fling and the transcript's animated scroll. A test fails the build on a bare `clickable`, on a
screen with no heading, and on an animation added without consulting it.

The terminal grid is the one surface where the obvious answer is wrong, and it has its own decision
record: [ADR 0010](./docs/adr/0010-the-grid-is-described-not-read-out.md).

### 5.5 Four themes, one token layer

Every colour, font family and radius goes through a token layer, with four themes defined against it.
The rule that keeps the other three one attribute away is the one most likely to erode, so it is
enforced by a test that scans the tree and fails on any literal declared outside the token layer
rather than being documented and hoped for.

Known hole, since a rule with an exemption should say so: the test exempts the `theme` package
itself, which is where a single hard-coded 16-slot ANSI palette lives, shared by all four themes. All
four themes are dark; there is no light variant.

## 6. Security posture, in one paragraph

Kampr's socket reach is **unrestricted code execution on every host it can see** — it types into live
terminals, and full command passthrough is the product rather than a hole. So authentication is in
the node rather than at a front door, which is the opposite of Collie's choice and is forced by the
tier ladder: the moment the recommended deployment is a hostname you own, a delegating model has
nothing holding the door. Devices are enrolled, revocable, and carry a role; a `readonly` device
receives every server-to-client message and is refused `input`, `answer` and `manage`. Revocation and
demotion bite on the socket that is already open — the session re-reads the device row on a two-second
interval and again before every write verb — because the control an operator actually uses runs in a
different process and only writes SQLite.

The full account, including what each rung does *not* protect against and the residual risks that are
known and real, is [`docs/08-threat-model.md`](./docs/08-threat-model.md). Read it before binding to
anything but loopback.

## 7. The mesh

**Status: landed late and unproven.** `crates/kampr-mesh` holds the handshake, transport, dial
supervision and the hub's shadow of a remote pane; the node carries the accept and relay paths and a
`[mesh]` config section; `kampr mesh invite|join|list|revoke|forget` exists. It arrived after most of
this document was written, it has unit coverage and no end-to-end test across two real hosts, and it
has never been run in anger. Check [`docs/06-audit.md`](./docs/06-audit.md) before believing any
particular part of it works.

Herdr contributes nothing here and blocks nothing: `herdr --remote` makes the *local* Herdr an SSH
client of a remote one and streams a UI to a terminal. There is no cross-host socket API.

The shape is **peers dialling outbound to a hub**. Only the hub needs an inbound path, which is what
lets a laptop behind NAT join without a port forward and lets an operator point one reverse proxy at
one hostname. "Hub" is a role a node is configured into, not a separate build. Once the handshake is
done the link carries the ordinary v1 client protocol *backwards* — the hub is a client of the peer,
sending `watch` and `input` — so there is no second protocol and the per-connection backpressure rule
applies at both hops by construction. Node identity is an ed25519 keypair generated on first run, and
mesh auth is entirely separate from user auth, so a compromised viewer session cannot impersonate a
node. [ADR 0007](./docs/adr/0007-peers-dial-outbound-to-a-hub.md).

Two things the mesh unlocks that are worth naming because they are the point of the whole exercise.
**Named sessions are separate Herdr servers** (#49), which is why a *node* in the wire model is a
herdr server rather than a machine: the configured session keeps the node's own id and every other
session on the host takes a suffixed one. Enumerating them gives a herd the TUI cannot show at once
even on a single host. And **splitting a view across instances needs no protocol support at all** —
each pane is an independent stream and nothing binds a view to one server, so one Kampr window can
show panes from three machines and two sessions. A Herdr TUI client attaches to exactly one server
and structurally cannot.

## 8. What is built, and what is not

This project's roadmap tick state has been unmaintained since its first commit and is not evidence of
anything. [`docs/06-audit.md`](./docs/06-audit.md) is the standing record, checked against code, and
this document does not duplicate it — but three things belong here because they are architectural
rather than a defect list:

- **The mesh landed very late and has no end-to-end test.** §7. Everything this document says about
  it describes intent that now has code behind it, not a capability anyone has exercised across two
  machines.
- **Notifications landed last**, which is backwards given that the entire point of a phone client is
  being *told* an agent is blocked. The one architectural fact worth carrying forward is why the
  event they rest on is awkward: `pane.agent_status_changed` cannot be subscribed the way every other
  event is. Herdr rejects it without a `pane_id`, and **one invalid entry rejects the whole
  `events.subscribe` call** (#54) — so it has to be subscribed per pane and re-subscribed whenever
  the pane set changes, and getting that wrong silently costs every other subscription too.
- **The stated payoff of node-side emulation has only partly been collected.** The argument in
  [ADR 0001](./docs/adr/0001-the-node-runs-a-vt-emulator.md) was that selection, find and hyperlinks
  become node features over a cell model rather than three client reimplementations. Hyperlinks did.
  Selection is implemented in the client. Find does not exist at all. With one client shipping the
  cost is zero; with a second it is the whole argument.
  [`docs/04-wire-protocol.md`](./docs/04-wire-protocol.md) still states the claim in full, and should
  be narrowed or the benefit collected.

## 9. Parked deliberately

Not planned, not scheduled — recorded so they are not re-discovered from scratch or acted on by
accident.

- **`terminal session control` in any form.** It always claims the PTY and there is no flag to
  decline (#17), and while a controller holds it the person at the desk is ignored (#18). This is the
  one entry on this list that is a hard rule rather than a parking spot.
  [ADR 0002](./docs/adr/0002-kampr-never-resizes-a-pane.md).
- **Recovering markdown structure by re-parsing the grid.** The information is not there to recover.
  [ADR 0005](./docs/adr/0005-structure-comes-from-the-transcript.md).
- **Reflowing terminal rows into wrapped text for phone width.** That is Collie's answer and a good
  one for Collie's product; the tier ladder and pan-and-zoom replace it here.
- **A VT emulator in Kotlin.** It runs once, in Rust, in the node.
- **A local shell on iOS.** Third-party apps cannot spawn processes and there is no JIT. iOS gets
  remote panes, and that is the honest answer. **Android is different** — an app may fork/exec and
  allocate a PTY inside its own sandbox, which is how Termux works, so a kampr node on Android could
  be a provider with Herdr not involved at all. It would run as the app's uid and see the app
  sandbox, not the device: useful, not root. The provider seam exists so that stays possible.
- **Requiring Tailscale for anything.** It is one rung on the ladder, alongside a reverse proxy with a
  real certificate.
- **Managing anybody's front door.** Kampr supplies TLS itself or trusts a proxy; it does not
  supervise a tunnel. Collie's ADR 0001 reasoning applies with more force here, because the ladder
  deliberately spans several front doors and Kampr could not test any of them.
- **Upstream asks that would delete whole sections of this document.** A subscribable output event
  would remove the polling; reflow rather than crop in `observe --cols` would remove the geometry
  trade entirely; `terminal.scroll` on an observer would remove most of §4.4; an offset parameter on
  `pane.read recent` would remove the rest. They are listed at the end of
  [`docs/02-roadmap.md`](./docs/02-roadmap.md) and are worth filing rather than working around.
