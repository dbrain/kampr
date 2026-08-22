# Kampr wire protocol v1

The contract between a Kampr node and a client. **Write code against this, not against Herdr** —
Herdr's frame format stops at the node and never reaches a client.

One WebSocket per client at `/ws`, JSON text frames, every message a tagged object with `t`.
Unknown `t` values and unknown fields are **ignored, not errors** — that is how v1 clients survive
v1.1 nodes.

## Why cells, not ANSI

The node runs one VT emulator per pane (`kampr-term`, `alacritty`-free, built on `vte`) and ships
clients a cell grid. Clients parse no escape sequences. Consequences that are load-bearing:

- One emulator per pane, shared by every viewer, not one per viewer.
- Selection, find, and OSC 8 hyperlinks are node features over a cell model, not three client
  reimplementations. Hyperlinks survive here where `pane.read` drops them (probe #36/#37).
- Zoom and pan are pure rendering. Kampr **never resizes a pane** (probe #17).

## Ids

`node_id` is a stable ULID chosen at `kampr init`. A pane's global id is `"<node_id>/<pane_id>"`,
where `pane_id` is Herdr's own (`w3:p2`). Clients treat it as opaque.

One host can run several Herdr servers — a named session is a whole separate server — and each is
its own node. The configured session keeps the bare ULID; the others are `"<ULID>.<session>"`, so
`"01J.../w3:p2"` and `"01J....agents/w3:p2"` are two different panes. **Match a node id exactly,
never by prefix**, and split a global pane id on the first `/`.

## Server → client

### `hello` — first message on every connection

**Clients decide what to show from `security`, never by guessing from the URL.** A passkey button
that cannot work must be absent, not present-and-failing — and `unlocks` is the copy for what a
hostname would buy (findings §3.7).
```jsonc
{ "t": "hello", "protocol": 1, "node_id": "01J...", "node_name": "comingclean",
  "build": "0.1.0+abc1234", "role": "full",          // "full" | "readonly"
  "caps": { "push": true, "scrollback": true, "conversation": true, "manage": true,
            "mesh": true },   // this node accepts peer links; see "The mesh"
  "security": {
    "tier": 0,                       // 0 = ip:port, 1 = hostname+cert, 2 = public, 3 = tailscale
    "encrypted": false,              // is this a secure context?
    "unencrypted_banner": true,      // show the persistent Tier-0 warning
    "passkeys": false,               // WebAuthn possible here? an IP is never a registrable domain
    "push": false,                   // needs a secure context
    "installable": false,            // PWA install / Add to Home Screen
    "unlocks": ["passkeys", "push", "installable"]   // what a hostname would buy
  },
  "push": { "key": "BMd9…" } }   // present only when caps.push; the VAPID applicationServerKey
```

**`caps.push` and `security.push` are different questions and a client needs both.**
`security.push` says whether the *origin* permits push at all — a secure context, which plain HTTP
on a LAN IP is not. `caps.push` says whether this *node* can actually send one: secure context
**and** a VAPID key **and** `push.enabled` in its config. Hide the affordance on either, and use
`security.unlocks` for the copy that says what a hostname would buy.

`push.key` is the application server key a browser needs before it may call
`pushManager.subscribe`. It is public, and it rides on `hello` to save a round trip on the one
path most likely to be interrupted. `GET /api/push` carries the same value for clients that want
it without a socket.

**The greeting is three frames, in this order: `hello`, `herd`, `prefs`.** The third is pushed
unasked so a client can restore per-pane zoom and view before it paints — see `prefs` below.

### `role` — this device's role changed mid-connection

```jsonc
{ "t": "role", "role": "readonly" }        // "full" | "readonly", the same two `hello` uses
```

The node re-reads the device behind every live socket, so a demotion or a promotion — from
`kampr setup`, from `POST /api/devices/{id}/role`, or from any other process holding the same
store — lands on the connection that is already open. **This frame is how the client is told.**
Without it a demoted device keeps every write affordance drawn — the key row, the New sheet, the
manage actions — and discovers the truth by pressing something and getting `error{not_writer}`;
a promoted one waits for a reconnect it has no reason to make.

- **It is not a second `hello`.** `hello` is the *first* message on a connection and stays that
  way. Nothing else in the greeting has changed, and a client that re-ran everything a greeting
  means would throw away its herd and its preferences over a permission change.
- **It carries only the role.** The device's id, name and expiry are as `hello` gave them; a
  change to those closes the socket instead (see `revoked`, below).
- **Sent on the change, not on a timer**, and only when the effective role actually moved. A
  client that never sees one is a client whose role never changed.
- **A client that does not know this `t` ignores it** and behaves exactly as it does today, which
  is the same forward-compatibility rule the whole protocol runs on.

Effective immediately in both directions: the node is already enforcing the new role by the time
this arrives, so a client must gate on it rather than on the role it was greeted with.

### `herd` — the whole model; sent after `hello` and on any reconnect
```jsonc
{ "t": "herd",
  "nodes": [ { "id": "01J...", "name": "comingclean", "kind": "local",   // "local"|"peer"
               "online": true, "rtt_ms": 0.4, "herdr_version": "0.8.2",
               "build": "0.1.0+abc1234",   // this node's kampr build — see "Version skew"
               "update": "0.1.2",          // ABSENT unless a newer release exists — see below
               "detail": null } ],   // why it is offline, when it is
  "panes": [ { "id": "01J.../w3:p2", "node_id": "01J...",
               "workspace_id": "01J.../w3", "tab_id": "01J.../w3:t1",  // node-qualified, usable as `at`
               "workspace": "kampr", "tab": "1", "cwd": "/home/dbrain/dev/kampr",
               "label": null,
               "agent": "claude",                    // null on a shell pane — picks the default view
               "agent_status": "blocked",            // idle|working|blocked|done|unknown
               "cols": 74, "rows": 30,               // rows: the PTY's own viewport, not the rect.
                                                     // cols: ABSENT until measured — see below
               "scrollback_rows": 0,                 // 0 = no ring (alt screen) or unsafe to read
               "has_conversation": true,             // a transcript for this pane resolves on disk
               "watchers": 2,                        // ABSENT below 2 — see below
               "updated_at": "2026-08-20T13:44:02Z" } ] }   // stamped by the node; Herdr's snapshot carries no time
```

`herd.patch` carries the same shapes under `added` / `changed` / `removed_ids`, **for nodes as
well as panes**. A herd going away is a node flipping to `online: false`, so a patch that only
ever carried panes left an outage invisible.

**A node is a herdr server, not a machine.** Every named session on a host is a separate herdr
server with its own socket (probe #49), so each is its own node: the configured session keeps the
node's own id, and the rest take `"<node_id>.<session>"`. Pane ids stay unique by construction,
and a client can tell `default` from `agents` without being told the rule. Ids are opaque — do not
parse the suffix.

**`online: false` is a real state a client must render, not an error path.** A node serves before
it can reach its herdr and after that herdr dies, so the first `herd` on a fresh connection may
carry an offline node and no panes at all. `detail` is the operator-readable reason (an optional
string, absent when the node is up) so the UI can say *why* the herd is empty rather than showing
an empty herd. Panes are **not** dropped for an outage — a herdr restart keeps them (probe #70) —
so a client marks them stale and keeps its cached grids.

When a node comes back the node re-sends the whole `herd`, which is the same recovery the
"sent after `hello` and on any reconnect" rule already describes.

### `styles` — append-only style table, per connection
```jsonc
{ "t": "styles", "from": 12,
  "styles": [ { "fg": {"k":"r","v":[255,120,0]}, "bg": {"k":"d"},
                "bold": true, "underline": true } ] }
```
`fg`/`bg` are `{"k":"d"}` default, `{"k":"i","v":n}` indexed 0–255, or `{"k":"r","v":[r,g,b]}`.
Boolean attributes are omitted when false: `bold dim italic underline blink reverse strike hidden`.
Style `0` is always the default pen. Ids are stable for the life of the connection.

**`cols` is the pane's real PTY width, which is not always its layout rect.** In a *headless*
session — what the plugin and the systemd unit both produce — the PTY does not follow the rect: a
split halves the rect and leaves the PTY alone (probe #68), and even unsplit the rect is a column
wider (#69). The node measures the width from `pane.read` rather than trusting the rect (#84/#85),
and `grid.reset.cols` always carries that measurement.

**`herd.panes[].cols` is therefore optional and is omitted when nothing has measured it.** A width
is proved by a soft wrap, and a pane nobody is watching has produced no proof — so an unwatched
pane carries no `cols` at all rather than the rect, which is a width no row was ever wrapped at
(measured: rect 47, PTY 93). A client shows the width it is given and shows nothing when there is
none; it never derives geometry of its own and never falls back to the rect. `rows` is always
present, because herdr reports the PTY's own viewport.

**`has_conversation` is "a transcript for this pane resolves", not "this harness has an
adapter".** A `claude` started a minute ago has written nothing yet, and a pane that claimed a
conversation it could not produce opened on a blank Conversation view whose `convo.load` answered
`not_found`. It can still never outrun `hello.caps.conversation`: a node with no adapter for a
harness resolves nothing for it.

**`watchers` is how many viewers the node is streaming this pane to, and it is omitted at 0 and
at 1.** A phone and the person at the desk could type into the same terminal with no sign of each
other; this is the sign. The common case is one viewer and costs nothing on the wire, so a client
reads an absent field as *just me* and must not distinguish absent from `1`.

It means **open, not typing.** A viewer is somebody with the pane on screen — nothing here knows
whether anyone's hands are on the keys, and a client must not say they are.

**It is a floor, never a headcount, and it never over-counts.** Three things it does not see:

- **The operator at the desk.** These are Kampr viewers. Somebody sitting at the herdr session
  itself is not one of them and is never counted.
- **Everyone behind a hub.** A mesh-relayed pane is watched through one `watch` per pane however
  many clients sit behind the hub, so a peer's pane carries **the peer's own number** — its local
  viewers plus one for the whole hub — and the hub republishes it unchanged. Two phones on a hub
  looking at the same peer pane are one viewer in that number.
- **A client that has not watched yet.** The count is of live `watch`es, so the `herd` a client
  receives on connect reflects the world *before* its own watch. Joining a pane somebody else has
  open produces a `herd.patch` a moment later; the opening `herd` is not where the badge comes
  from.

Under-counting is deliberate. A phone that claims company when there is none has told a lie about
a terminal, and there is no reading of this field that recovers from that.

### `grid.reset` — full repaint; drop any prior state for this pane
```jsonc
{ "t": "grid.reset", "pane": "01J.../w3:p2", "cols": 74, "rows": 30,
  "rows_data": [ /* RowDiff */ ],
  "cursor": { "col": 37, "row": 9, "visible": true },
  "links": [ "https://herdr.dev" ] }
```

### `grid.patch` — only the rows that changed
```jsonc
{ "t": "grid.patch", "pane": "01J.../w3:p2",
  "rows": [ { "row": 9, "runs": [ {"s": 0, "x": "❯ 1. "}, {"s": 4, "x": "Yes"} ] } ],
  "cursor": { "col": 5, "row": 10, "visible": true },
  "links": [ "https://herdr.dev" ] }        // delta only, appended to the pane's table
```

**RowDiff** is `{ "row": <u32>, "runs": [ Run ] }`; **Run** is `{ "s": <style_id>, "x": "<text>",
"l": <link_id?> }`. Runs are contiguous from column 0 and cover the full row width; trailing default
cells may be omitted, and the client pads with blanks.

`row` is `u32`, not `u16`: on `grid.*` it is a viewport row, but on `scrollback` it is an absolute
ring index, and a deep ring overflows 16 bits.

**`links` may appear on `grid.patch` as well as `grid.reset`, and the two carry different things.**
A hyperlink can first be seen mid-stream, so a client that only reads `links` from `grid.reset` will
render link ids it was never given. But the two messages are not the same shape:

- **`grid.reset` carries the whole table.** Replace the pane's table with it.
- **`grid.patch` carries only the suffix** — the entries discovered since the last message. Append it.

Ids are indices into the resulting table. Treating a reset as an append is not a cosmetic bug: after
the second reset every id is off by the length of the previous table, so links silently resolve to
the **wrong URL** rather than failing visibly.

Measured compression against per-cell JSON: **61×** at 124×50 with 49 distinct pens (4,205 vs
257,985 bytes) and **44×** at 74×30 with light colour (1,769 vs 78,471). The ratio holds; the
absolute size scales with how much colour is on screen.

### `scrollback` — history above the viewport, oldest first
```jsonc
{ "t": "scrollback", "pane": "01J.../w3:p2", "from_top": 0,
  "rows": [ /* RowDiff, row = absolute index from the top of the node's ring */ ],
  "total_rows": 171, "complete": true, "capped": false }
// total_rows is a DEPTH, not a highest index: the ring spans from_top .. from_top + total_rows.
```
Only sent for panes with `scrollback_rows > 0`. Sourced from `pane.read recent format=ansi` and run
through the same emulator, so styling matches the live grid. Agent panes never have this — their
history is the conversation.

**Scrollback is delivered as one document then tails.** The first `scrollback` after a `watch` carries
what the node holds; as the ring grows under a live watcher, further `scrollback` messages carry only
rows above what was already sent, keyed on absolute row index. A client appends by index and never
assumes a message is the whole ring.

**A backpressure purge must never drop a `scrollback` or `styles` message.** History is append-only
and nothing repairs a hole in it, and a purged style entry orphans runs that survive. Only
`grid.reset` and `grid.patch` are purgeable — the reset that follows a purge restores them both.

**There is no `scrollback.load`, deliberately.** Herdr caps a scrollback read at **1000 lines** and
`pane.read` has no offset parameter (probe #51), so a client cannot page further back and neither can
the node — asking again just returns the same newest 1000. What the node *can* do is accumulate: while
it is watching, successive reads overlap, so it stitches them into a ring that grows past the cap.
History that scrolled away before the node started watching is unreachable, and `capped: true` says
so rather than pretending the top of the ring is the top of history.

**On a gap, the node discards what it held rather than keeping it behind a hole.** If output outruns
the poll — more than 1000 rows between reads — the new read shares no overlap with the ring, so the
two stretches are not adjacent and nothing can prove what sits between them. Splicing them would make
`from_top` and `total_rows` fiction. The node drops the old rows, advances `from_top` by their count
so absolute indices stay true, and sets `capped`.

That is a real loss, and a `cat` of a large file or a verbose build will cause it. Two things follow:

- **The node polls adaptively**, faster while a pane is producing output, so gaps stay rare rather
  than being accepted as normal. This is the mitigation; the discard is the honest floor beneath it.
- **Preserving history across a gap needs a wire change, and is deliberately not in v1** — it would
  take either a per-segment `from_top` or a gap sentinel row, and both should be specified before
  anyone implements them. Decide it on evidence that gaps still hurt after adaptive polling.

A **width change** restarts the ring for a different reason: every stored row was wrapped at the old
width, so nothing older can be trusted to line up. Same restart, distinct cause, and the log says
which happened.

### `convo` / `convo.turn` — transcript-derived, agent panes only
```jsonc
{ "t": "convo", "pane": "01J.../w3:p2", "cursor": "opaque", "more": true,
  "turns": [ { "id": "t_812", "role": "assistant", "at": "2026-08-20T13:41:55Z",
               "blocks": [ { "b": "md", "text": "Six, and they are…\n\n| Key | … |" },
                           { "b": "tool", "name": "Bash", "summary": "probe key grammar",
                             "lines": 48, "state": "done" },
                           { "b": "code", "lang": "ts", "text": "send(pane, \"\\u001b[5~\")" } ] } ] }
```
`b` is `md` | `code` | `tool` | `diff`. Markdown is passed through verbatim — the **client** renders
it, so tables stay tables.

A `tool` block's `state` is `running` | `done` | **`error`**. A tool turn is **revised in place** when
its result lands: match by turn id and replace, never append, or every tool renders twice.

**A `diff` block is not one dialect.** Claude sends a unified diff rebuilt from `structuredPatch`;
Codex sends its `*** Begin Patch` envelope verbatim. Both share `+`/`-` line prefixes, so one
classifier covers them, but a renderer must not assume unified-diff headers are present.

**Turn order is the node's order.** Do not sort by `at`: a resumed session carries records whose
timestamps predate the ones above them, and sorting shuffles the conversation. Replace by id, keep
the given order.

### `pending` — a prompt is waiting
```jsonc
{ "t": "pending", "pane": "01J.../w3:p2", "question": "Do you want to make this edit?",
  "options": [ { "key": "1", "label": "Yes" }, { "key": "2", "label": "Yes, and don't ask again" } ],
  "source": "transcript" }              // "transcript" | "screen"
```
`source` records where the question came from. Claude does **not** write a pending tool request to its
transcript before approval, so its questions come from the screen; Codex does, so an unmatched tool
call is its signal (probes #42, #43). **Clients must not care which.**

**A prompt is cleared by the same message with `question: null, options: []`.** There is no separate
"resolved" message — a client should treat null as "no prompt outstanding" and hide the strip.

### `notified` — the answer to a `notify`
```jsonc
{ "t": "notified", "ok": false, "reason": "no_foreground_client", "pane": "01J.../w3:p2" }
```
`ok: false` is the common case, not an error: a *headless* herdr session — what the plugin and the
systemd unit both produce — has no attached client to show a toast to, and says so (probe #77).
A client that reports "told the desk" without checking is reporting something that did not happen.

### `error`
```jsonc
{ "t": "error", "code": "not_writer", "message": "this device is read-only", "pane": null }
```
Codes: `not_writer` · `unknown_pane` · `node_offline` · `herdr_unavailable` · `bad_request` ·
`not_found` · `revoked` · `unsupported` (the node does not implement that op).

There is no `rate_limited` error code. Toast throttling answers `notified{ok:false,
reason:"rate_limited"}` on its own frame and pairing throttling is an HTTP **429**, so nothing ever
emitted one and a client written to expect it was written against a code that does not exist.
Relayed errors are the one exception to this list being closed: a hub forwards a peer's `code`
verbatim, so a newer peer's code reaches a client unchanged rather than being dropped.

`herdr_unavailable` and `node_offline` are both emitted for a herd outage: the first names the
cause on the connection, the second says which node went with it, and an op addressed at a pane on
a node that is down is refused with `node_offline` rather than left to time out.

`code` is an open string, not a closed enum: a client must handle an unrecognised code by showing
`message` rather than failing. `revoked` is one such: the node re-reads the device behind a live
socket, so a revocation or a Tier 0 expiry lands on the connection that is already open rather
than at the next handshake. `revoked` is followed by a close. A demotion to `readonly` is read on
the same path but is **not** an error and does not close anything — it arrives as `role`.

## Client → server

```jsonc
{ "t": "watch",   "pane": "01J.../w3:p2", "scrollback": true, "conversation": true }
{ "t": "unwatch", "pane": "01J.../w3:p2" }

// Input. Exactly one of text/b64/keys. text and b64 go to pane.send_text;
// `pane.send_text` takes a JSON string, so bytes that are not valid UTF-8 have no
// representation on the wire to Herdr. b64 is a convenience for control characters,
// NOT a raw-byte escape hatch: invalid UTF-8 is rejected with bad_request rather than
// mangled. Every escape sequence Herdr's key grammar rejects is UTF-8-safe, so nothing
// the key row needs is lost.
// keys goes to pane.send_keys and is limited to Herdr's grammar (probe #7).
// Anything Herdr's grammar rejects — Home, End, PageUp, PageDown, Insert, Delete — is sent as
// its escape sequence through text/b64 instead (probe #8/#9). Clients should prefer text.
{ "t": "input", "pane": "01J.../w3:p2", "text": "\u001b[5~" }
{ "t": "input", "pane": "01J.../w3:p2", "b64": "G1s1fg==" }   // must decode to valid UTF-8
{ "t": "input", "pane": "01J.../w3:p2", "keys": ["ctrl+c"] }

{ "t": "answer",      "pane": "01J.../w3:p2", "key": "1" }
// The NODE decides whether a submit key follows, per harness — Claude selects on the digit alone,
// Codex needs Enter (probe #43). A client sends only the key it was offered in `pending.options`.
{ "t": "convo.load",  "pane": "01J.../w3:p2", "before": "opaque" }
// Per-pane, per-device preferences — zoom level, view choice, render mode. The node stores them
// against the device, so they follow you between browsers on the same enrolled device.
{ "t": "prefs", "pane": "01J.../w3:p2", "prefs": { "zoom": 1.6, "view": "terminal" } }
//   -> { "t": "prefs", "panes": { "01J.../w3:p2": { "zoom": 1.6, "view": "terminal" } } }
// A write with no `pane` (or no `prefs`) stores nothing and just asks for the current set.
// `pane` must be a pane this node is serving (`unknown_pane` otherwise) and the *stored* blob must
// fit in 2 KiB (`bad_request`). Unbounded rows under an arbitrary id is a disk-fill, whatever the
// role, so the bound is on the write rather than on `readonly`. A node keeps at most 256 panes'
// preferences per device and drops the least recently updated first.

// A toast on the *operator's desktop* (probe #50). `title` is required; `pane` picks which herdr
// session shows it and defaults to the node's own. The node prefixes this device's name — an
// unattributed toast on somebody's screen is a phishing surface, and a client is exactly as
// attacker-influenceable as pane output — strips control characters, rate limits to one per five
// seconds per connection, and audits it. `readonly` devices are refused with not_writer.
{ "t": "notify", "pane": "01J.../w3:p2", "title": "Taking this pane", "body": "from the phone" }
//   -> { "t": "notified", "ok": true, "reason": null, "pane": "01J.../w3:p2" }

{ "t": "resync" }                       // node replies with herd + grid.reset for every watched pane
{ "t": "ping", "n": 7 }                 // -> {"t":"pong","n":7}
```

There is **no resize message and there will not be one.** The node cannot reshape a pane.

**A `prefs` write is a merge, not a replacement.** It names the keys it is changing and leaves the
rest of that pane's blob alone, so a client that stores a zoom does not thereby forget the view.
`null` as a value **removes** that key — with a merge there is no other way back. Two writes:

```jsonc
{ "t": "prefs", "pane": "01J.../w3:p2", "prefs": { "view": "conversation" } }
{ "t": "prefs", "pane": "01J.../w3:p2", "prefs": { "zoom": "2" } }
//   -> { "t": "prefs", "panes": { "01J.../w3:p2": { "view": "conversation", "zoom": "2" } } }
{ "t": "prefs", "pane": "01J.../w3:p2", "prefs": { "view": null } }
//   -> { "t": "prefs", "panes": { "01J.../w3:p2": { "zoom": "2" } } }
```

**The node pushes `prefs` unasked, as the third frame of the greeting** — after `hello` and `herd`,
on every connection, whether or not anything is stored (`{"panes":{}}` when nothing is). There is no
other way for a client to learn the zoom it left a pane at, and a client that has to ask has already
painted the pane at the wrong size. A client must therefore expect a `prefs` frame it did not
request, and must not treat the first one on a socket as the answer to its own write.

Values are opaque to the node: it stores and returns whatever JSON it is given, so `"1.6"` and
`1.6` both round-trip and a client should read either.

## Ordering and recovery

- Per pane the node sends exactly one `grid.reset` before any `grid.patch`, and re-sends `grid.reset`
  after any gap it cannot patch across (observer restart, herdr reconnect, native geometry change).
- Geometry changes arrive as a fresh `grid.reset` with new `cols`/`rows`. Herdr's own `full:true`
  frames map to this one-to-one, and cost nothing: only the first frame of a stream is `full`
  (probe #53).
- **The node polls to notice a desk resize.** No Herdr event fires when the attached client resizes —
  verified across six event types and three confirmed geometry changes (probe #52). `layout.updated`
  covers structural change only. Clients see the same `grid.reset` either way.
- **Reconnect is cheap by construction**: a full grid is ~3 KB and Herdr coalesces bursts to end
  state (probe #23/#25), so there is never a backlog. Clients render their cached last grid
  immediately, marked stale, and swap on the `grid.reset`. No spinner.
- The node drops a slow client's `grid.patch` queue and sends one `grid.reset` instead of buffering.

## The terminal surface always fills the viewport

A client must never letterbox a pane. Blank space below the last row is a bug, not a layout.

- **Scrollback and the live grid are one continuous surface**, not two panels. History scrolls up out
  of the top; the live viewport sits at the bottom and stays pinned there unless the user scrolls
  away. There is no divider and no separate scrollback mode.
- **Default zoom fills at least one axis.** Compute `max(fit-width, fit-height)` and pan the other,
  rather than `min(...)`, which is what produces letterboxing. A user may zoom further out, but the
  default never leaves a margin.
- **When a pane has no ring** (alt-screen, so `scrollback_rows == 0`) there is nothing above the grid
  to fill with, so fill-height wins and the surface pans horizontally. Those panes default to the
  conversation view anyway, which scrolls naturally.
- **No fixed row budget in the UI.** The node's ring bound is a memory limit, not a display one, and
  it is configurable — a client must not impose its own cap on top.
- Full bleed on every breakpoint: the terminal reaches the edges, with chrome floating over it rather
  than insetting it.
- **Paint bleeds; content insets.** These are two different rectangles and conflating them is what
  produces either a letterbox or an unreadable row hidden under the key row. The terminal *paints*
  edge to edge, so rows run under the header and the key row and nothing is ever blank. The
  *scrollable content* is inset by the chrome, so the pinned last row settles clear of it and no row
  is permanently obscured. Same semantics as safe-area insets on a phone, and the reason chrome can
  stay opaque.
- **Fill is computed against the paint rectangle**, not the inset one — otherwise the insets
  reintroduce the letterbox they exist to avoid.
- **Selection is by long-press and drag**, with handles at each end and a floating Copy action —
  the same idiom as selecting text anywhere else on the phone. Selection is **linear** by default,
  flowing across intervening rows like a paragraph; block selection is a secondary mode.
- **Copied text is the logical text, not the painted grid.** Strip each row's trailing padding, and
  join rows that are a soft wrap of one logical line. A path or URL copied with a newline through the
  middle of it is worse than not copying at all.
- **Links are tappable, and the data is already there.** A cell carries `link` into the pane's link
  table, so an OSC 8 hyperlink is a real harness-declared URI — one that `pane.read` drops and the
  frame stream preserves (probes #36, #37). Bare URLs in cell text are detected too, but **detected
  is not declared**: match a strict scheme conservatively, run detection over *logical* lines so a
  URL wrapped at the grid edge is not missed, and never auto-navigate. Pane output is
  attacker-influenceable.
- **Paste must supply its own bracketed-paste framing.** `pane.send_text` writes raw bytes with no
  framing (probe #9), so a multi-line paste without `ESC[200~` / `ESC[201~` executes line by line in
  a shell instead of arriving as one block.
- **Tapping the grid raises the keyboard.** No button summons it: a "show keyboard" control is a
  workaround for focus not working, not a feature. The tap gesture must lose to drag, so panning the
  grid does not toggle the keyboard on every flick.
- **Touch targets are 44 dp in portrait and 36 dp in landscape.** The 44 dp guideline assumes a
  one-handed reach; a landscape key row is a two-thumb precision posture at the screen edges, and
  44 dp there costs a quarter of a 390 dp-tall screen. The second landscape row is also collapsible,
  with the state remembered — a user who wants maximum terminal can drop the symbols row without
  losing the inverted T.
- **The key row docks flush to the keyboard**, with no space between them — it is an accessory to the
  keyboard and reads as part of it. Track `visualViewport.height + offsetTop` live through the
  show/hide animation; laying out against `window.innerHeight` or a fixed inset produces exactly the
  gap this rule exists to forbid. On Android use the animated `WindowInsets.ime`, not
  `navigationBars`, which leaves a static gap the height of the nav bar.


## Herd management

Everything you would do at the keyboard. Additive to v1 — a node that does not implement these
replies `error{code:"unsupported"}`, and a client hides what a node's `hello.caps` does not claim.

`hello.caps` gains `"manage": true` when the node exposes them.

```jsonc
// Structure. `at` is a pane, tab or workspace id depending on the verb.
{ "t": "manage", "op": "workspace.create", "node": "01J...", "label": "kampr", "cwd": "~/dev/kampr", "env": {} }
{ "t": "manage", "op": "tab.create",       "at": "01J.../w3",    "label": "tests", "cwd": "~/dev/kampr" }
// `at` for tab.create is a WORKSPACE id. Nodes accept a tab id too and derive its workspace.
{ "t": "manage", "op": "pane.split",       "at": "01J.../w3:p2", "direction": "right", "ratio": 0.5, "cwd": null }
{ "t": "manage", "op": "pane.zoom",        "at": "01J.../w3:p2", "mode": "toggle" }
{ "t": "manage", "op": "rename",           "at": "01J.../w3:p2", "label": "build" }   // null clears, panes only
{ "t": "manage", "op": "close",            "at": "01J.../w3:p2" }                     // pane | tab | workspace
{ "t": "manage", "op": "focus",            "at": "01J.../w3:p2" }

// Agents. `kinds` come from the node, not a hardcoded client list — 20 on a typical host.
{ "t": "manage", "op": "agent.start", "at": "01J.../w3:p2", "kind": "claude", "name": "reviewer", "args": [] }

// Worktrees — Herdr has first-class git support and it maps straight through.
{ "t": "manage", "op": "worktree.create", "node": "01J...", "cwd": "~/dev/kampr", "branch": "feat/x", "base": "main" }
{ "t": "manage", "op": "worktree.open",   "node": "01J...", "path": "~/dev/kampr-feat-x" }

// Layouts. `layout` is Herdr's own nestable split tree, opaque to the client.
{ "t": "manage", "op": "layout.export", "at": "01J.../w3:t1" }
{ "t": "manage", "op": "layout.apply",  "at": "01J.../w3:t1", "layout": { } }
// `layout` is either Herdr's `layout.export` reply verbatim or just its `root` node; both accepted.

// Named sessions are separate Herdr servers, so this one shells out on the node rather than
// calling a socket method. Same shape to the client; only the node knows the difference.
{ "t": "manage", "op": "session.create", "node": "01J...", "name": "agents" }
{ "t": "manage", "op": "session.stop",   "node": "01J...", "name": "agents" }
```

`workspace` and `tab` on a pane are **labels for a human**; `workspace_id` and `tab_id` are what a
`manage` op's `at` takes. A pane id carries its workspace (`w3:p2` → `w3`) but never its tab, so
without `tab_id` a client cannot address `tab.rename`, `tab.close` or `tab.focus` at all.

A `manage` message may carry an opaque **`rid`**, and the node echoes it verbatim on the
`managed` ack. It is additive and optional: a browser with one op in flight has no use for it, and
a hub relaying several clients' ops down one link does. A node that receives no `rid` sends none.

A refused op is acknowledged too: `{"t":"managed","op":…,"ok":false,"code":…,"message":…}`, followed
by the ordinary `error` frame. A client waiting on an ack must therefore watch `ok`, not just arrival.
**Every** refusal is acknowledged, including the ones the node decides before it looks at the op —
a read-only device's `not_writer`, an unreadable op's `bad_request`, and a target on a node this
herd does not serve — and each carries the `rid` it was sent with. A refusal that arrived as an
`error` alone left a client's in-flight state set forever.
`layout.export` puts the exported tree on its ack as `layout`.

Every `manage` op is acknowledged with `{"t":"managed","op":…,"ok":true,"id":"<new id, when one was created>"}`
and the resulting structure change arrives as an ordinary `herd.patch`. `id` is a **node-qualified
container id** — a workspace, tab or pane — for every op that creates one. The two exceptions are
`session.create` and `session.stop`, whose `id` is the bare session **name**: a session is named
rather than addressed, and dressing its name up as `<node_id>/<name>` produced something shaped
exactly like a pane id that no client can watch. Clients must not
optimistically mutate their herd model — wait for the patch, so the node stays authoritative.

`readonly` devices are refused every `manage` op with `not_writer`.

**Content-Security-Policy.** The node serves a strict CSP with **no external origins** — everything a
client needs must be bundled. It includes `'wasm-unsafe-eval'` (required by Skiko/wasm, and strictly
weaker than `'unsafe-eval'`) and `worker-src 'self' blob:`.

### Capability discovery

`served` says whether this node is *serving* that session as a node of its own. A session can be
running and unserved — an operator may restrict the set — and a client must not offer to open a
pane on a session that will never appear in the herd.

```jsonc
{ "t": "caps", "node": "01J...",
  "agent_kinds": ["claude", "codex", "gemini", "…"],   // from server.agent_manifests, not hardcoded
  "sessions": [ { "name": "default", "running": true,  "served": true },
                { "name": "agents",  "running": false, "served": false } ] }
```

### A note on splits

A `pane.split` changes the Herdr layout, so it changes pane geometry **for everyone**. That is
correct for an explicit action and does not conflict with "Kampr never resizes a pane" — that
invariant is about side effects of *viewing*, not about refusing to act. Say what it will do before
doing it.

Kampr's own **split view is a different thing entirely**: a client-side mosaic of independent
`observe` streams that may come from different sessions on different nodes. The Herdr TUI cannot do
that, because a TUI client attaches to exactly one server. It needs no protocol support beyond
watching several panes at once.


## The mesh

Additive to v1 in the strictest sense: **the mesh introduces no new client-facing message.** A
client sees one herd with more nodes in it, and every node in that herd is addressed exactly as a
local one — `"<node_id>/<pane_id>"` was already the id, so the routing was already in the ids.

### The shape, and why

- **Peers dial outbound to a hub.** Only the hub needs an address, so a laptop behind NAT joins
  with no port forwarding and an operator points one reverse proxy at one hostname. A peer that
  cannot be reached inbound is fully usable.
- **Hub is a role, not a build.** `[mesh] accept = true` — **off by default** — is the entire difference.
  Any node can hold peers, dial hubs, or both; a hub may itself be a peer of another hub.
- **The link carries this protocol, backwards.** After the handshake the *hub* is the client: it
  sends `watch` / `unwatch` / `input` / `answer` / `manage` / `ping` and receives `hello` / `herd` /
  `styles` / `grid.*` / `scrollback` / `pending` / `managed` / `pong`. The peer serves it with the
  same session code that serves a browser, so every rule below — the role gate, the bounded queue,
  the purge policy, the audit line — applies at the mesh hop because it is the same code.

### `GET /mesh` — the peer transport

A WebSocket, subject to no device token and no `Origin` check, because the credential is inside it.
Mesh authentication is a **mutual ed25519 handshake** and is a separate credential space from
anything a browser holds: a compromised viewer session has a bearer token, and the mesh handshake
never asks for one.

```jsonc
// peer → hub, first message on the socket
{ "t": "mesh.hello", "protocol": 1, "node_id": "01J...", "node_name": "laptop",
  "build": "0.1.0+abc1234", "key": "<hex ed25519 public key>", "nonce": "<hex 32 bytes>",
  "join": "K7QF-9M2X" }        // only on the connection that enrols this node; omitted after
// hub → peer
{ "t": "mesh.challenge", "protocol": 1, "node_id": "01J...", "node_name": "front",
  "build": "0.1.0+abc1234", "key": "<hex>", "nonce": "<hex 32 bytes>" }
// peer → hub
{ "t": "mesh.auth", "sig": "<hex ed25519 signature>" }
// hub → peer, then the v1 stream begins
{ "t": "mesh.accepted", "sig": "<hex>", "enrolled": false }
// or, and then the socket closes
{ "t": "mesh.refused", "code": "unenrolled", "reason": "…" }
```

Both signatures cover the same transcript, each under its own role label:

```
peer\n                       (or "hub\n")
KAMPR-MESH/1
hub-key=<hex>
peer-key=<hex>
hub-nonce=<hex>
peer-nonce=<hex>
hub-node=<id>
peer-node=<id>
```

Both keys bound in means a signature cannot be replayed at a third node; both nonces bound in means
it cannot be replayed at the same one; the version bound in means a later protocol cannot be
negotiated down to this one; and the role label means a signature made as a peer can never be
presented as a hub's. Refusal codes: `unenrolled` · `revoked` · `bad_signature` · `bad_join_code` ·
`wrong_hub` · `protocol`.

**Order matters and is deliberate.** The hub verifies the signature *before* it consults enrolment,
so a stranger learns nothing about whether a key would have been accepted; and it signs only after
it has decided to accept, so a stranger never collects a hub signature over a transcript it chose.
The peer checks its pin *before* it signs, so a stranger answering at the hub's address does not
collect a peer signature either.

**Why not mTLS or Noise.** mTLS is the obvious answer and it does not survive the deployment: a
reverse proxy terminates TLS, so a client certificate never reaches the node, and the whole point
is to sit behind one. Noise would add end-to-end confidentiality through that proxy, which is worth
having when the proxy is on another host — but it needs a second key type alongside the ed25519
identity that already exists, and TLS to the proxy plus a loopback hop is the documented topology.
So: TLS supplies confidentiality, ed25519 supplies mutual authentication, and the transcript is a
signature over a challenge in the same shape as SSH's `publickey` auth. If the proxy hop is ever
untrusted, Noise_IK under this same handshake is the upgrade, not a redesign.

### Enrolment and revocation

- `kampr mesh invite` on the hub mints a **single-use join code**, short-lived, rate limited, and
  in a different table from device pairing codes — one enrols a browser, the other enrols a node,
  and neither is redeemable for the other.
- `kampr mesh join --hub <url> --code <code> [--fingerprint <fp>]` on the peer. The hub's key is
  **pinned**; a different node answering at that address afterwards is refused with `wrong_hub`.
  With `--fingerprint` the pin is confirmed before this node signs anything; without it, it is
  trust on first sight and the CLI says so.
- `GET /api/mesh` lists peers and hubs with fingerprint, `online`, `rtt_ms` and build.
  `POST /api/mesh/{id}/revoke`, or `kampr mesh revoke`, cuts one off — and ends the link that is
  already open rather than waiting for the next handshake.
- A hub also gets a **device row** on each peer, so it appears in that peer's device list and is
  revocable there. No token is ever minted against it, so nothing can present it as a bearer.

### Relay, and backpressure per hop

The hub keeps **one** `watch` per pane per link however many clients are looking at it, and a
**shadow** of that pane's cell grid — the last state the peer published, not a second emulator.
There is still exactly one emulator per pane and it runs on the node that owns it.

The shadow is what makes the backpressure rule hold at this hop without a second mechanism:

- **Peer → hub.** The peer's outbox is the ordinary bounded client queue. A hub that falls behind
  has that pane's queued patches dropped and gets one `grid.reset`. Frames are already coalesced to
  grid state by Herdr, so a reset costs one full grid (~4 KB) and never a backlog.
- **Hub → client.** Same rule, same code, and the reset is served **from the shadow** — so a purge
  costs no round trip to the peer at all, and a second client joining a peer pane renders
  immediately from memory.
- **Hub fan-out.** A client that overruns the hub's per-pane channel is caught up with one
  `grid.reset` from the shadow rather than a queue it can never drain.

Nothing at either hop may purge a `scrollback`: history is append-only, keyed on absolute row
index, and no later reset repairs a hole in it. `pending` is likewise a fact about the pane rather
than a repaintable frame. Only `grid.reset` and `grid.patch` are droppable, exactly as on a local
pane.

Scrollback is stitched at the hub by absolute index. If a peer's ring restarted — a gap it could
not bridge, or a width change — the hub **discards what it held** and advances `from_top` rather
than splicing two stretches that are not adjacent, and sets `capped`. That is the same honesty rule
the node applies locally, applied a second time.

### Version skew and latency

`rtt_ms` on a `kind: "peer"` node is the **mesh link's own measured round trip**, from the hub's
`ping` to the peer's `pong`, plus that node's own herdr round trip. On a `kind: "local"` node it is
the herdr socket ping. So a peer on a slow link reads as slow, and a client that renders `rtt_ms`
shows the number that explains it.

`build` on each node is that node's kampr build. Two nodes in one herd may be running different
releases; a client can only say so if each node names its own, and this is that field.

`update` is the release that supersedes that node's `build`, as a version rather than a flag — a
boolean says a machine is stale without saying what it is stale against. **It is absent whenever
there is nothing to say**, and a client renders all of those cases identically, which is not at
all:

- the node is on the latest release;
- the node could not reach GitHub, so it does not know;
- the node's build is not a version this can compare (a working copy between two tags);
- its operator set `[update] check = false`, and the node never asked.

**Each node answers for itself; a hub never judges a peer.** Only the node knows what it is
running, and only its own config can say whether it may ask GitHub at all — a hub that filled the
field in for a silent peer would be publishing a judgement produced by a request that peer's
operator declined. A hub re-publishes a peer's entry verbatim, so `update` travels the mesh on the
same path as `build`.

The check runs at most once a day, and its cadence is held on disk (`update.json` in the state
directory) rather than in the process, so a node under a supervisor that is restarting in a loop
is not a request in a loop. A failed check is retried in an hour, keeps the last good answer, and
never surfaces as `detail` — an unreachable GitHub is not a fault of the node.

A change to `build` or `update` is an ordinary `herd.patch`, because the check lands long after
the `herd` a client was greeted with.

**Nothing on this protocol installs anything.** There is no message that updates a node, and no way
for a hub to push a binary to a peer — `kampr update`, typed on the machine itself, is the only
path. A node that can type into every terminal on its host does not replace its own binary unasked,
and a hub that could push binaries would turn one compromised machine into code execution on every
machine the operator owns, which is a far larger blast radius than the pane access this protocol
already grants.

### When a peer drops

Its node flips to `online: false` with a `detail` saying so, delivered as an ordinary `herd.patch`.
**Its panes stay listed** — dropping them empties a node out of the UI at the moment the user most
needs to see that it exists and is unreachable. A live watcher gets
`error{code:"node_offline"}` on the pane; a new `watch` on it gets the same. Every other node in
the herd, including the hub's own, is untouched: a link owns nothing but its own tasks.

Recovery is unattended. The peer reconnects on its own backoff, its node flips back to `online`,
and clients see a fresh `herd` and a `grid.reset` per pane.

## Notifications

Additive to v1. Everything here is HTTP rather than socket messages, because a browser hands its
push subscription to the page as JSON and a service worker — which has no socket — has to be able
to re-register one on its own (`pushsubscriptionchange`).

A client shows nothing at all unless `hello.caps.push` is true. See `docs/08-notifications.md`.

```jsonc
// What this device may do, and what it has already asked for.
GET  /api/push
//   -> { "available": true, "key": "BMd9…", "secure_context": true, "unlocks": ["passkeys"],
//        "subscribed": false,
//        "endpoints": [ { "kind": "webpush", "endpoint": "https://…" } ],
//        "rules": [ { "pane_id": "01J.../w3:p2", "muted": true, "snooze_until": null } ] }

// The browser's own PushSubscription.toJSON(), plus a `kind`. `kind` is "webpush" from a browser
// and "unifiedpush" from an Android distributor — a label for the device list, never a branch:
// UnifiedPush 3.0 is RFC 8291, so both are delivered to identically.
POST /api/push/subscribe
//   { "endpoint": "https://…", "kind": "webpush", "keys": { "p256dh": "…", "auth": "…" } }
//   -> { "subscribed": true }
// The endpoint must be https and is upserted on the endpoint, not the device: a browser that
// re-subscribes gets the same endpoint back, and a stale row under another device would
// double-send. A device keeps at most 8.

POST /api/push/unsubscribe
//   { "endpoint": "https://…" }   -> { "subscribed": false, "removed": true }

// Snooze and mute, per agent and per device. `pane_id: "*"` covers every agent on this device.
// A rule that neither mutes nor snoozes deletes the row rather than storing a no-op.
POST /api/push/rules
//   { "pane_id": "01J.../w3:p2", "muted": false, "snooze_until": 1793000000 }
//   -> { "rules": [ … ] }

// Warm resume. The service worker fetches this while the notification is still being read, so
// the tap that follows opens onto data rather than a load.
GET  /api/warm?pane=01J.../w3:p2
//   -> { "t": "herd", "nodes": [ … ], "panes": [ … ], "role": "full",
//        "pending": { "t": "pending", "pane": "…", "question": "…", "options": [ … ] } }
```

**A `readonly` device may subscribe.** Being told an agent is blocked is *reading*, and it is the
whole point of a device you half-trust with a screen.

**A revoked device stops being woken immediately**, because `push_targets` is a join against live
devices in the same database — revocation is a `WHERE` clause, not a cleanup job.

### The push payload

What a service worker receives, after the browser has decrypted it. Versioned, because a service
worker outlives the page that registered it and may be older than the node sending to it.

```jsonc
{ "v": 1,
  "title": "claude · kampr needs you",
  "body": "Do you want to make this edit?",   // THE QUESTION, not just which agent
  "tag": "kampr.blocked",                     // one tag: the newest replaces, never stacks
  "count": 1,
  "pane": "01J.../w3:p2",                     // null on a batch — a tap opens the triage list
  "panes": [ { "pane": "01J.../w3:p2", "node": "01J...", "agent": "claude",
               "label": "kampr", "question": "Do you want to make this edit?" } ] }
```

**Simultaneous blocks are one notification.** A 900 ms window opens at the first block; everything
inside it lands in one payload, split per subscription so a device that muted one of three agents
sees the other two rather than the whole batch or nothing.

**The body carries the question because the node already has it.** It is extracted for the
`pending` message off the screen (probe #42), so a notification that only named the agent would be
withholding what the node holds. On Android the OS may hold the app long enough that a tap arrives
before the socket is up, which is exactly when a body that says something useful earns its keep.

## Auth

The WebSocket carries a device token as a `Sec-WebSocket-Protocol` subprotocol of the exact form
**`kampr.token.<token>`**, echoed back verbatim by the server. The
node resolves it to a device and a role before `hello`. A client that sends any other subprotocol
spelling fails the handshake. `readonly` devices get every server → client
message and are refused `input` / `answer` with `not_writer`. HTTP endpoints for enrolment
(`/auth/pair`, `/auth/webauthn/*`) are specified alongside the auth work, not here.

A pairing code is **not on its own a credential once any device is enrolled.** `kampr setup` prints
it into a Herdr popup pane, and a `readonly` device receives every frame of every pane — so a code
minted for the console stays inert until an operator arms it from that console, for a short window.
The keypress is the channel a watching device does not have. A code handed back over an
authenticated `POST /api/pair` was never on a screen and is armed as it is minted.
