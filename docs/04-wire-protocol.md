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
- Zoom and pan are pure rendering and reshape nothing (probe #17). The one op that reshapes a pane
  is `pane.size`, which an operator asks for by name — see [ADR 0012](adr/0012-one-deliberate-resize-behind-a-panel.md).

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
  "build": "0.1.21", "role": "full",          // "full" | "readonly"
  "caps": { "push": true, "scrollback": true, "conversation": true, "manage": true,
            "mesh": true },   // this node accepts peer links; see "The mesh"
  "device": { "id": "01J...", "name": "pixel", "expires_at": 1788000000 },
                                     // expires_at is epoch seconds, or null for a device that does not expire
  "security": {
    "tier": 0,                       // 0 = passkeys impossible here, 1 = passkeys possible
    "origin": "http://192.168.1.24:8790",   // the canonical origin the tier was decided from
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
               "build": "0.1.21",          // this node's kampr build — see "Version skew"
               "update": "0.1.2",          // ABSENT unless a newer release exists — see below
               "detail": "…" } ],   // ABSENT when the node is up — see below
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
               "detail": "…",                        // ABSENT when it has a picture — see below
               "cmd": "cargo",                       // the foreground job's name — ABSENT far more often than not
               "argv": "cargo test",                 // its whole command line — OFF by default, see below
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

**`cmd` and `argv` are what a pane is *doing*, and their absence is the normal case.** `dir-name`
stops identifying anything once six panes share a directory, so the node reads herdr's
`pane.process_info` for every pane and puts the foreground job on the wire: `cmd` is the process
name (`cargo`), `argv` the whole command line with its arguments (`cargo test`), and a pipeline is
its members joined with ` | ` (`sleep 9 | cat`, probe #297). Both are omitted when there is no job,
never sent as `null` or `""`.

**`argv` is off unless an operator turns it on, and `cmd` is not.** A command line carries
`mysql -phunter2`, `curl -H "Authorization: …"`, `ssh -o ProxyCommand=…` — and every paired device
receives the herd model, **`readonly` included**, at `hello` and on every patch, with no `watch`
involved. That is not the same disclosure as the screen a readonly device can already stream: the
model arrives unasked, an alt-screen or cleared pane shows nothing while `argv` names the job for
its whole life, and what a client holds is stored state rather than pixels. A device you
half-trust with a screen is not one you meant to hand a password. So `argv` is absent unless
`[naming] send_argv` says otherwise, and `cmd` — the process name, which is all the naming problem
ever needed — is always sent.

**This costs no client release.** The shared default template's `{argv|cmd}` group falls through to
`cmd` when `argv` is absent, which is exactly what it already does under ble.sh below. A client
that reads `argv` must simply treat it as absent by default, like every other optional field.

There are two ordinary reasons for no job and neither is a fault. A pane sitting at its prompt is
running its shell, which is not an answer to "what is this pane doing". And on a machine that
sources **ble.sh** — every interactive shell on the operator's own — the job runs inside the
shell's own process group, so herdr reports the shell however busy the pane is (probe #297). **A
client must therefore treat `cmd` as legitimately missing most of the time**: render what is there
and drop the section when there is nothing, rather than drawing an empty one. `kampr (cargo test)`
becomes `kampr`, never `kampr ()`. That is what `kampr_core::naming`'s `[…]` construct exists for,
and the same engine ships in `client/shared` so the CLI, the phone and the web agree on the name —
`crates/kampr-core/tests/fixtures/naming-cases.json` is what holds the two implementations to each
other.

A name Kampr computes can also be written *into* herdr, where it draws on the pane's border at the
desk (probe #294). That is a node-side setting (`naming.report_to_herdr`), **off by default**,
because marking somebody's screen because a phone is looking at it is the side effect
[ADR 0002](./adr/0002-kampr-never-resizes-a-pane.md) exists to refuse. Nothing about it appears on
this wire.

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

**`panes[].detail` is why this pane has no picture, and it is absent whenever it has one.** The
pane-level twin of `nodes[].detail`, in the operator's own words, for the state that is neither
"the herd is up" nor "the herd is down".

A node reaches Herdr **two** ways and can have exactly one of them working: a socket for the
model, and a spawned `herdr terminal session observe` for the screens. A node whose PATH has lost
the binary serves a correct herd, accepts input, answers every health check — and streams nothing.
Every symptom points at the half that works. What that looked like on a phone was a blank grid
with a flashing cursor, in every pane, for ever, on two of three machines for months (probe #233).

- **It is a state, not an event.** The supervisor behind it retries for ever, so the field clears
  itself the moment a pane can paint again — a `herd.patch` with the entry and no `detail`. A
  client renders it for as long as it is there and takes it down when it goes.
- **It is node-scoped even though it rides on panes.** A spawn that fails is the configured binary
  missing or not executable, which no pane can cause and no pane can fix, so every pane of that
  node carries the same reason rather than only the one somebody happened to open.
- **A pane with no `detail` is not a pane with a picture.** An empty pane is an ordinary thing — a
  brand-new workspace's really is nearly blank (probe #212) — and this field never says so.
- **Additive.** A client that has never heard of it behaves exactly as it does today, and the
  arrival is announced on the frame it already understands: `error{stream_unavailable}`.

**No `grid.reset` is sent for a pane in this state.** The geometry is a promise that rows are
coming, and it is made *after* the stream exists rather than before — a client that has been told
`74×30` has laid out a grid, drawn the caret and started waiting.

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
"l": <link_id?>, "w": <1|2?>, "m": [ "<marks>" ]? }`. Runs are contiguous from column 0 and cover
the full row width; trailing default cells may be omitted, and the client pads with blanks.

`row` is `u32`, not `u16`: on `grid.*` it is a viewport row, but on `scrollback` it is an absolute
ring index, and a deep ring overflows 16 bits.

**`w` is columns per character in `x` — 1 or 2, omitted when 1 — and it is the only thing that
makes a row's columns countable.** A double-width glyph occupies two columns, in herdr's cell
model and therefore in the node's: herdr advances two columns for one and addresses the next glyph
at col+2 (probe #210). A row's width is `Σ codepoints(x) × w`, **not** the length of the joined
text, and the second column of a wide glyph carries the lead's style and link so a background or an
underline spans the whole glyph.

Three consequences a client has to implement, not derive:

- **Count code points, not UTF-16 units.** An astral glyph is one cell. A `CharArray` puts its two
  surrogate halves in two cells and renders neither.
- **The column after a wide glyph belongs to that glyph.** Hit testing, selection ends, the caret
  and the offset a link detector is given all resolve to the *lead* column, or they are a glyph
  out. Column and string offset are two coordinates and stop agreeing here.
- **`x` is exactly one code point per cell.** That is why the width rides on a field rather than
  on a sentinel character in the run — nothing has to remember to strip anything — and it is the
  rule `m` exists to protect. For a cell wearing nothing, which is nearly all of them, `x` is also
  exactly the text on screen.

A run breaks on a width change as it does on a style change, so `AB日本語CD` is three runs.

**`m` is what each cell of the run is wearing on top of its base, and it is why `x` can stay one
code point per cell.** A cell is a grapheme, not a code point: herdr keeps a combining mark, a ZWJ
and a variation selector on the base they belong to and addresses the next glyph at base + the
*cluster's* width (probe #223). Putting the whole cluster in `x` would break the one thing that
makes a row countable, so the marks travel beside the text instead:

```jsonc
{ "s": 0, "x": "rese", "m": ["", "\u0301", "", "\u0301"] }   // ré-su-mé, four cells, four columns
{ "s": 0, "x": "\ud83d\udc68", "w": 2, "m": ["\u200d\ud83d\udc69\u200d\ud83d\udc67"] }  // one family, two columns
```

- **One entry per cell, by position, and the list is truncated after the last marked cell.** An
  entry is `""` where a cell wears nothing, and a run with no marked cell at all omits `m`
  entirely — which is nearly every run, so the field costs nothing in the common case.
- **`m` never changes the arithmetic.** A row is still `Σ codepoints(x) × w` columns wide, a run
  still breaks only on style, link and width, and a marked cell never splits a run.
- **It is additive.** A client that has never heard of `m` renders the bases it already rendered,
  with the columns where they were — which is exactly the behaviour of every node before this
  field existed. Nothing about `x`, `w` or `s` was reinterpreted.
- **The tail column of a wide cluster carries no marks.** They belong to the lead, and a client
  that read both halves would draw them twice.
- **Copy, find and link detection read `x` *and* `m`.** `x` alone is the bases; the text on screen
  is each cell's base followed by its marks. A client that already keeps a cell buffer has both.

**What the node clusters.** A cell is a UAX #29 extended grapheme cluster, because herdr's is.
Measured against the column herdr wraps a string at on a 93-column pane: Devanagari
`\u0915\u094d\u0937` moves to the next row whole while the same shape in Tamil splits, which is
GB9c to the letter and not something a handful of hand-written rules reach (#225). So zero-width
code points join the cell to their left, and so do a spacing mark, the character after a prepend,
a skin-tone modifier, a conjoining jamo block however many lead jamo it stacks, the second of a
flag's two regional indicators, and a pictograph after a ZWJ — but **only** a pictograph:
`X\u200dY` is two cells and so is `\u65e5\u200d\u672c`, exactly as herdr leaves them. A mark
printed with no cell to its left is dropped, as herdr drops it, and so is a zero-width space,
which is not a mark and has nothing to ride on.

**The column count is herdr's, not `unicode-width`'s.** A cluster spends `min(width, 2)` columns,
and a cluster that starts with a regional indicator spends **2** whether or not it found its pair.
Both diverge from `unicode-width` 0.2.2 and both were measured (#226, #227): `\u1100\u1100` sums
to 4 columns and herdr spends 2, three prepends before a digit sum to 4 and herdr spends 2, and a
lone `\U0001f1eb` sums to 1 where herdr spends the 2 a whole flag gets. A variation selector that
widens its base to two columns takes the second column with it, the same way.

**What is still out, and why it is unreachable rather than unfixed.** Herdr discards a cluster it
cannot attach to anything — a bare jungseong or jongseong jamo, a choseong or jungseong filler,
`A\ufe0f`, `e\ufe0e`, a leading combining mark — before it ever reaches the node, so the node
keeps some of those where herdr would have dropped them. No herdr frame and no `pane.read` can
carry one, so nothing on the wire can show the difference. Two more, same reason: herdr renders a
cluster it has no glyph for as blank cells in its `observe` frames, so a live grid can show `AB  CD`
where scrollback shows the jamo — the columns agree, the content is herdr's to lose, not the
node's. And a cluster that grows into a second column while it is already sitting in the row's
last column stays one column wide instead of moving to the next row whole; herdr wraps it, but
herdr also wraps every row before the node ever parses it, so the node is never handed that shape.

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
it, so tables stay tables. A `diff` block carries `text` and an optional `path`. `cursor` is
**absent** when there is nothing further back to ask for, which is not the same as `more: false`.

`role` is `user` | `assistant`, and `user` means **a person typed this**. Every harness writes some
of its own text into a user record — a background agent finishing, a slash command's envelope, the
environment block codex opens a session with — and across every transcript on this machine that is
45% of what claude files under a user role ([#286](./03-probe-log.md)). A node strips those before
it serves a turn and drops a turn that was nothing else, because a client has no way to tell them
apart and two things break if it tries: the reader is told they said something they did not, and a
view that groups an exchange between one prompt and the next cuts an answer in half wherever a
notification landed in the middle of it.

`at` is the harness's own stamp, copied through. It is optional, and where it is present it is
**RFC 3339 with an explicit zone** — every adapter in tree writes UTC and says so with a `Z`
([#285](./03-probe-log.md)), which is what lets a client draw it as a time of day in the reader's
own zone rather than as an age. A node adding an adapter must keep that: a stamp with no offset is
a floating local time, and the only honest reading of one is an elapsed time. Clients must treat
the zone as part of the value and must not assume UTC when it is absent.

#### `fresh` — this page replaces, it does not merge

A page **merges by id**: turns whose ids the client already holds are replaced in place, and each
turn it does not hold goes **where the page puts it** — after the last id the page and the client
have in common, before the next one. When they share nothing at all, the page is prepended whole.
That is what lets `convo.load` page backwards through one transcript. It is also what leaves a
conversation the pane has *left* sitting underneath the one it moved to, because the new
transcript's ids match nothing.

Unconditional prepending was the older rule, and clients that still do it are on real phones. It is
correct for `convo.load`, which pages backwards, and wrong for a transcript re-read after its pump
was restarted, which pages forwards: the turns the client is missing are then the *newest* ones and
every one of them lands at the top of a conversation that is scrolled to the bottom — never seen,
and never revisited, because the node has recorded them as delivered. **A node must therefore never
send a merging page carrying a turn newer than what the client holds.** Where the node knows the
client is already showing this transcript, the turns go out as `convo.turn`, which appends; where
the overlap is gone entirely, the page says `fresh`.

Where the node knows what a client is holding it takes the old turns off by name — a `convo.turn`
carrying their ids and no blocks, exactly as a live preview is retired. There are cases where it
cannot: a client that reconnects arrives on a socket the node has no history for, and a node that
restarted has no history for anybody. Then the page says so.

```jsonc
{ "t": "convo", "pane": "01J.../w3:p2", "fresh": true, "cursor": "opaque", "more": true, "turns": [ … ] }
```

- **`fresh: true`** — *drop every turn you hold for this pane, then apply this page.* It is the
  pane's whole conversation as far back as one page reaches; `more`/`cursor` still page further.
- **Absent or `false`** — merge, as before. Every page `convo.load` answers with is one of these:
  they are older slices of the transcript already on the screen.

**Additive.** A client that has never heard of the field merges, which is what every build did
before it — including the case this field exists for, so an older phone is no worse off than it is
today. A hub relaying a peer's pane sets it on the **first** page it hands each client for that
pane, because only that hop knows which of its clients has been sent one; a `fresh` the peer set is
never cleared.

#### `att` — an attachment's header, never its bytes

An `md` block may carry an optional `att`. It is **additive**: the `text` beside it is the marker an
installed client already renders — `[image · png]` — so a client that has never heard of the field
goes on showing exactly what it shows today.

```jsonc
{ "b": "md", "text": "[image · png]",
  "att": { "id": "opaque", "kind": "image", "mime": "image/png", "bytes": 52831, "name": "shot.png" } }
```

- **`id`** is required and opaque. It is minted by the node that served the turn and it resolves at
  the route below, on **that** node and for **that** pane. Do not parse it, do not store it as a
  permanent handle, and expect it to stop resolving: it names a record in a transcript, and a pane
  that moves to a different transcript takes every id with it. An id whose record has since been
  rewritten answers `404`, not different bytes — the size the header quoted is carried inside the
  id and checked against what comes back. A client that wants the picture *after* that has happened
  builds an id of its own from the path the tool call named; see "A second id form" below.
- **`kind`** is an **open string**. `image` and `file` are the two a node yields today. A client that
  does not recognise one must treat it as a file and offer a download — never drop the block — so a
  later `video` needs no protocol change and no client release.
- **`mime`, `bytes`, `name`** are optional and are **absent when the source did not carry them**, not
  empty. A pasted screenshot has no filename and no dimensions; the media type is all there is
  (probe #248). `bytes` is the exact decoded length, computed from the record's own base64 — Claude's
  `originalSize` beside it is the size of the *file it read*, which disagrees with what is in the
  record 300 times out of 499 (#249).

**The bytes never travel on this websocket, and that is the point.** The same socket is carrying live
terminal frames for every pane the client is watching, and the largest single attachment measured is
**2.22 MB in one record** (#247). Pushing that down the socket head-of-lines every pane on the
connection for as long as a phone link takes to drain it — for a screenshot the operator may never
open. So the header goes on the wire, the operator decides, and the bytes come over HTTP once.

#### `GET /api/attachment/{pane}/{id}` — the bytes

Authorised exactly like every other `/api/*` route: a device token in `Authorization: Bearer …`.
There is no second way in, and **a read-only device may fetch one** — looking at a screenshot
somebody pasted into an agent session is reading.

A pane id is `<node_id>/<local>` and the slash is sent **literally, not percent-encoded**, so the
path is three segments and not two:

```
GET /api/attachment/01JNODE/w3:p2/att-7f3
```

Exactly three. A path of any other shape is refused rather than guessed at, because guessing wrong
about which part is the pane anchors every check below on the wrong pane.

```
200  Content-Type: image/png
     Content-Length: 52831
     Content-Disposition: inline                       // attachment; filename="…" for anything else
     Cache-Control: no-store
404  { "error": "no such attachment" }
413  { "error": "this attachment is larger than the node will serve" }
```

- **`404` is one answer on purpose.** An id that escapes the transcript root, one for a different
  pane's transcript, one whose record has since been rewritten and one that was never valid are not
  distinguishable from outside.
- **A `200` always has a body.** An attachment with no bytes in it is a `404`; there is no such
  thing here as an empty success.
- **The recorded media type decides what the node shows, never what the node *is*.** It is a string
  an agent wrote into a file, so it is served back only from a list of image types the node is
  willing to render inline; anything else — `text/html`, `image/svg+xml` — becomes
  `application/octet-stream` with an `attachment` disposition. SVG is deliberately not on that list:
  it is a scriptable document wearing an image's name.
- **When the record names no media type at all, `Content-Type` is answered from the bytes.** A paste
  may carry none (#248), and `Content-Type` is what a client names a saved file from. The sniff can
  only ever produce a type off the same list, so it widens what is shown without widening what is
  trusted — and a media type the record *did* name is never second-guessed.
- **The ceiling is 8 MiB decoded**, read off the record's base64 *before* anything is allocated, so a
  record claiming a gigabyte costs a comparison. It is between three and four times the largest
  attachment ever measured (#247).
- **A pane on another node is served from the peer that owns it, over the link that peer dialled.**
  The hub has no inbound path to a peer ([ADR 0007](./adr/0007-peers-dial-outbound-to-a-hub.md)), so
  it cannot fetch this over HTTP; it asks on the mesh link instead and streams the answer back. The
  response is the same shape a local one has, including the ceiling and the single `404`, and the
  `Content-Type` is the **hub's** decision from the same allowlist — a peer that recorded
  `text/html` gets `application/octet-stream` here too.
- **A hub advertises the attachment only while it can serve it.** It strips every `att` off the
  `convo` and `convo.turn` messages it relays for a peer's pane whenever that peer is offline,
  unknown to this hub, or running a build whose `hello` does not claim `attachments` in its
  `caps` — leaving the image marker's text and no button. A client attached to the peer directly
  always gets both.

#### A second id form: a path on the node's filesystem

An `att.id` minted by a node names a **record** in a transcript, and that is what stops resolving
when the transcript is rewritten under it — the picture is still on disk, and the id for it is not.
So the route also answers a second form of id, and **a client builds this one itself**: it saw a
path in a tool call and wants what is at it.

```
id = base64url-no-pad( "file" U+001F <absolute path> )
```

`U+001F` is the same unit separator the record form has always used, and `base64url-no-pad` is the
same alphabet — an id is still one path segment with nothing in it a URL minds. Nothing else is
tagged: the record form is five separator-delimited fields and has been since the first build that
minted one, so the **number of fields** is what tells them apart and every id an installed client is
holding decodes to exactly what it decoded to before. This is additive in the direction that
matters, too: a node older than this feature answers `404` for a file id, which is what a client
must already handle.

```jsonc
// "/var/lib/kampr/shot.png"
"ZmlsZR8vdmFyL2xpYi9rYW1wci9zaG90LnBuZw"
// "~/shot.png" — resolved against the serving node's home, see below
"ZmlsZR9-L3Nob3QucG5n"
```

- **Only a device that may send input may ask for one**, and a read-only device gets the ordinary
  `403 this device is read-only` with an audit line. The reasoning is the whole of the security
  argument here: a device that can type into a terminal can already `cat` any file the node's user
  can read, so a path read is no escalation *for it* — and it is a real one for a device you
  half-trust with a screen, whose whole point is that it cannot reach `~/.ssh/id_rsa`. The refusal
  is decided from the id's own shape, before anything on disk is looked at, so it says nothing
  about the path. **The record form keeps its looser gate**: a read-only device may still fetch a
  screenshot out of a transcript.
- **A leading `~/` resolves against the node's own home**, and only a leading one. Agents write
  `~/screenshot.png` and `~/dev/x/plot.png` constantly and those are the paths a person taps, so
  `~/` and a bare `~` are expanded before anything else is decided. **`~user/x` is not** — guessing
  at another account's home would hand over a different user's files under a gate that reasoned
  about this one, so it falls through as a relative path and is refused. A `~` anywhere but the
  first character is an ordinary character in a filename: `/tmp/a~b` is served as itself. The
  separators straight after `~` belong to the prefix, so `~//etc/hosts` is the home's own file and
  never `/etc/hosts`. The home is the node's configured journals home — the *operator's*, not the
  process's, on a node running as a service user — and a node with no home at all resolves `~/x`
  to nothing and refuses it.
- **Beyond that gate there is no allowlist and no confinement.** `/etc/hosts` is fine, symlinks are
  followed, and there is no working directory in the question — which is why a **relative path is
  refused** rather than resolved against whatever directory the node happens to have been started
  in. Expansion is never what makes a path absolute: `foo/bar`, `./x` and `../x` are refused
  whatever the home is. A directory, a path that is not there, a fifo, and a file this user cannot
  read are all the same `404`, so the route cannot be used to map the filesystem by response code.
- **The pane in the URL still says which node serves it**, and nothing else. A file id on a peer's
  pane is carried over the mesh link exactly as a record id is, and the peer applies the same gate
  to the hub's own device before it reads anything.
- **`kind` and `mime` come off the extension and nowhere else.** A known image extension is
  `kind: "image"` with that type; anything else — including `.svg` and `.html` — is `kind: "file"`
  with no type, which is a download. A file with no extension has its `Content-Type` answered by
  the same sniff a record with no recorded media type gets, so a screenshot written without one
  still renders. `name` is the file's own name.
- **The ceiling is the same 8 MiB**, read off `stat` before a byte is allocated, and the route
  answers it whole rather than streaming: an 8 MiB body costs about **5 ms and 11 MiB of peak RSS**
  on the machine the file is already on (#258). It is not raised for this form either — the same
  `Fetched` crosses the mesh lane, and a peer buffers the whole body before it chunks it (#247,
  #257), so raising it is a measurement on that lane rather than on this route.

### `att.*` — the mesh link only

Five messages carry an attachment from a peer to the hub relaying it. They are **not part of the
client protocol**: a browser has the route above, and the reason that route is HTTP is that a 2.22
MB record (#247) must not share a queue with terminal frames. A node answers `att.*` for a hub and
ignores it from anything else, exactly as it ignores any other unknown `t`.

```
hub  → peer  { "t":"att.fetch", "rid":7, "pane":"01JNODE/w3:p2", "id":"att-7f3", "window":4 }
peer → hub   { "t":"att.open",  "rid":7, "bytes":52831, "kind":"image", "mime":"image/png",
                                          "name":"shot.png" }
peer → hub   { "t":"att.chunk", "rid":7, "seq":0, "b64":"…" }        // ≤ 64 KiB decoded
hub  → peer  { "t":"att.more",  "rid":7, "n":1 }
peer → hub   { "t":"att.end",   "rid":7 }
peer → hub   { "t":"att.error", "rid":7, "code":"not_found" }        // or "too_large", "busy"
hub  → peer  { "t":"att.stop",  "rid":7 }
```

- **`rid` correlates**, the same way `manage`/`managed` does. A frame naming an `rid` nothing is
  waiting for is dropped, not an error: an `att.stop` and the chunks it cancels cross in flight.
- **The window is the whole of the flow control.** The peer may send `window` chunks before it is
  asked for more, and the hub grants one back for each chunk it has handed downstream — so the hub
  holds the window and never the record, and it pulls at the rate the client is reading at. A peer
  that runs past its window is cut off.
- **Chunks go down a lane every other frame overtakes.** A pane on the same link repaints during a
  transfer rather than after it, and `att.end` rides that lane too or it would arrive first.
- **The ceiling is enforced on `att.open`'s claim before a chunk is asked for**, and again on the
  arithmetic as they arrive: a peer that sends more than it announced is cut off, and one that sends
  less ends the body short of its `Content-Length`.
- **`att.error` is one answer for the same reason `404` is.** `too_large` is the hub's `413`;
  everything else, including a peer that is offline or never answers, is the hub's single `404`.
- **`caps.attachments`** on a peer's `hello` is what tells a hub this build answers `att.fetch` at
  all. Absent means no, which is what every build before this one says.
- **`id` is whatever the hub was handed**, forwarded unchanged — a record locator or a file id. The
  peer decides which it is and applies its own gate: a file id is answered only for a hub whose
  device may send input *there*, and a demoted one gets `not_found` like any other refusal.

A `tool` block's `state` is `running` | `done` | **`error`**. A tool turn is **revised in place** when
its result lands: match by turn id and replace, never append, or every tool renders twice.

**A `diff` block is not one dialect.** Claude sends a unified diff rebuilt from `structuredPatch`;
Codex sends its `*** Begin Patch` envelope verbatim; `agy` sends the unified diff its edit tool
puts in the tool *result*, hunk headers and all but no `---`/`+++`. All three share `+`/`-` line
prefixes, so one classifier covers them, but a renderer must not assume unified-diff headers are
present.

**Turn order is the node's order.** Do not sort by `at`: a resumed session carries records whose
timestamps predate the ones above them, and sorting shuffles the conversation. Replace by id, keep
the given order.

#### Which conversation a pane has

**A working directory is not a session.** Every run of a harness in one writes its own transcript,
so "the newest transcript declaring this cwd" is somebody else's conversation as often as it is
this pane's — the run that was quit a minute ago, a `claude -p` from a shell, the pane next door.
Two field reports were the same defect wearing two faces: a pane whose agent was restarted kept
showing the session that was quit, and a pane that had just started its first agent showed
whatever had last been run in that directory.

A node resolves a pane's transcript from three handles, strongest first, and serves **nothing**
when none of them lands:

1. **`pane.agent_session`**, when herdr populates it and it agrees with the pane's own harness.
   Herdr 0.8.2 never does (probe #75), so this is a path that has to exist without ever firing.
2. **The harness process in the pane**, from herdr's `pane.process_info`. Claude 2.1.236 and later
   writes `~/.claude/sessions/<pid>.json` naming the session that pid is on, and removes it when
   the session exits, so this is exact — and it is the handle that moves the view when an agent is
   quit and a fresh one started in the same pane. `procStart` in that file is field 22 of
   `/proc/<pid>/stat` verbatim, and a node checks it, because a pid the kernel has handed on is
   not the process the file was written for. `/clear` rewrites `sessionId` in that file **in
   place**, under the same pid, so this handle follows a cleared session as well as a restarted
   one — a node re-reads it rather than trusting what it resolved at `watch`.

   `agy` 1.1.18 publishes the same map through the kernel instead of through a file: it holds an
   `flock` on `~/.gemini/antigravity-cli/presence/<conversation-id>.lock` for as long as that
   conversation is the one it is on, and `/proc/locks` names the pid holding it. The lock *files*
   are never unlinked, so the directory alone is a list of conversations that have **ended**; only
   the kernel's answer separates the live one. It needs no start-time check — a lock is released
   when its holder dies, so a pid that holds one is alive — and it moves with `/new`, which
   `~/.claude/sessions/<pid>.json` has no equivalent of. `/proc/locks` is world-readable, which
   `/proc/<pid>/fd` is not.
3. **The working directory, bounded by when that process started.** For the harnesses that
   publish no such map — Codex, and Claude before 2.1.236 — a transcript whose last record
   predates the pane's harness cannot be that harness's. It is a bound, not an identity: two runs
   in one directory at the same time are still indistinguishable this way.

   **`agy` has no third handle at all.** Its transcript declares no working directory, and the two
   caches that map one to a conversation — `cache/last_conversations.json` and
   `conversation_summaries.db` — are written on the way out, so while a conversation is live they
   name the one before it. A pane whose `agy` cannot be reached through handle 2 has no
   conversation, which is the supported state below.

**A pane whose harness the host looked for and did not find has no conversation**, rather than
falling back to the directory. Herdr detects a harness by scraping the screen, so a pane can claim
`claude` while nothing named `claude` is running in it; that is information, not the absence of
it. `has_conversation` goes `false` and the tab is hidden, which is a supported state.

**An empty conversation view is a missing feature; the wrong transcript is the tool lying about
what an agent said.** A node prefers the first.

#### The live turn — `id: "live"`

A harness writes an assistant record only when the message is **finished**, so a conversation built
from the transcript alone shows nothing until the turn ends. While a pane is `working`, the node
also reads the message off the pane's own screen and publishes it as an ordinary `convo.turn`
revision under the reserved id **`live`**.

```jsonc
{ "t": "convo.turn", "pane": "01J.../w3:p2",
  "turns": [ { "id": "live", "role": "assistant",
               "blocks": [ { "b": "md", "text": "The parser is a state machine over…" } ] } ] }

{ "t": "convo.turn", "pane": "01J.../w3:p2",
  "turns": [ { "id": "live", "role": "assistant", "blocks": [] } ] }   // withdrawn
```

Three rules, and they are the whole contract:

- **It is an approximation, and it always loses.** The screen is hard-wrapped to the viewport, has
  had its markdown rendered away, and is clipped at the top once a message outgrows the pane. The
  node withdraws it the moment the transcript carries the same message, so the authoritative record
  replaces it rather than sitting beside it. A client must never page from it, cite it, or keep it
  across a `convo` reload.
- **A turn with `blocks: []` is withdrawn, not empty.** Clients drop it from the rendered list —
  rendering it leaves a blank card where the preview was. `live` is the id it happens to most
  often, but **any** turn may be withdrawn: when a pane moves to a different transcript the node
  withdraws the previous conversation before sending the new one's first page, because a page
  *merges* by id and the ids of another session match nothing — so without the withdrawal the new
  turns arrive above the old ones and the panel reads as though it never updated.
- **It is always the newest turn**, it carries no `at`, and it is never a `cursor`. Everything else
  about it is an ordinary turn: match by id, replace, keep the given order.

Rendering it identically to a recorded turn is wrong in one specific way — the wording may still
change under the reader — so a client should mark it. Kampr's own conversation view puts a caret
and *still writing* under the text.

**A harness with no probed screen publishes no live turn**, which is the same conversation this
protocol described before live turns existed. Today that is `claude` 2.1.239 and `codex` 0.149;
every other agent kind serves its transcript and nothing more.

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

### `paste`
```jsonc
client -> node  { "t": "paste", "pane": "01J/w1:p1", "b64": "<base64>", "name": "shot" }
```

Bytes for the pane's agent to work on. The node writes them to a file **on the pane's own machine**
and types the path in as ordinary `input`; there is no reply of its own, and what the operator sees
is the path appearing in the composer.

**The path is the whole point.** An agent reached over ssh reads a local file perfectly well — it is
the terminal's own image-paste protocol that dies, so nothing here tries to speak one.

`name` is a hint at the *stem* only, and it is the only thing the client has any say in. The node
owns the directory, and the **extension is derived from the bytes**, never from what the sender
called them: an extension decides what the harness on the far end will do with the file, so a body
claiming `png` while holding something else is the entire shape of the problem. A name that is a
path cannot climb out of the directory it is joined to — separators, `..` and a leading dot are
dropped rather than escaped.

Gated exactly as `input` is, in the same dispatch-time device refresh, because it **is** typing: a
device that may not type is refused with `not_writer`. A pane on a peer is relayed rather than
written here — the file has to land on the machine the harness will read it from.

Ceiling **8 MiB**, checked against the base64's own length before anything is allocated, and refused
with `bad_request` rather than truncated. A pasted file is swept after a day, and the directory is
capped, so a pane that never reads its paste does not keep the bytes for ever.

### `convo.facets`
```jsonc
node -> client  { "t": "convo.facets", "pane": "01J/w1:p1", "facets": {
  "title":   { "text": "the width inference rewrite", "source": "manual" },
  "timings": [ { "turn": "<turn id>", "duration_ms": 315990, "messages": 144 } ],
  "queued":  [ { "text": "and copy the config across", "at": "2026-08-28T02:10:59.658Z" } ],
  "mode":    { "mode": "normal", "permission": "bypassPermissions" },
  "compactions": [ { "trigger": "manual", "pre_tokens": 756165, "post_tokens": 18709 } ]
} }
```

What the harness wrote down about the **session** rather than about any one turn. Sent unasked,
**once, when a conversation opens** — collecting it is a whole-transcript read (154 ms for 29.4 MB
measured), which a conversation opening can afford and a poll cannot, and none of it changes at the
follow rate.

**Every field is optional and the whole object may be `{}`.** Kampr serves three harnesses and the
wire is additive for ever, so nothing here is named after the record one harness happens to write:
a facet is filled only where a harness has been *measured* to carry an equivalent, and a harness
with nothing to say says nothing. Today Claude fills all five and Codex and agy fill none — their
nearest candidates were looked at and rejected as unmeasured rather than guessed at. A client draws
nothing for what it does not get.

`title.source` is `manual` or `generated`, and it is the whole point of carrying the source at all:
a name a person typed outranks one a harness made up, however good. `timings[].turn` is a turn id
the client already holds, so a duration hangs off the turn it belongs to.

### `convo.sub`
```jsonc
client -> node  { "t": "convo.sub", "pane": "01J/w1:p1", "id": "<handle>", "before": null }
node -> client  { "t": "convo", "pane": "01J/w1:p1", "sub": "<handle>", "fresh": true, "turns": [...] }
```

A page of a conversation the pane's agent **launched**, named by the handle a `sub` block carried.

**The node then follows it.** A subagent's transcript grows while it runs and the reason to open
one is to watch it work, so what it grows by arrives as `convo.turn` carrying the same `sub`:

```jsonc
node -> client  { "t": "convo.turn", "pane": "01J/w1:p1", "sub": "<handle>", "turns": [...] }
```

One at a time, and only while the reader has it open: asking for another replaces it, leaving the
pane replaces it with nothing, and a client that never opens one costs the node nothing. `sub` is
absent on the pane's own turns and always was, so an installed client only ever receives the frame
it already understood.

A turn carrying `sub` is **appended**, replaced by id — not merged the way a page is. A page runs
backwards and files what the reader does not hold above what they do; a transcript still being
written runs forwards, and merging it the page's way puts the newest step above the ones taken
before it. That is the same distinction `convo` and `convo.turn` already carry for a pane's own
conversation.

Its own verb rather than a field on `convo.load`, so a node that has never heard of it ignores the
frame exactly as it ignores any other unknown `t`. The page it answers with is an ordinary `convo`
frame wearing one additive field, `sub`, and it is **always `fresh`** — a launched conversation
shares no turn id with the pane's own, so there is nothing for a client to merge it into. A page
without `sub` is the pane's own and looks precisely as it always did.

`id` is opaque, minted by the node that served the turn, and resolved by handing it back. It is
**not** a path and must not be built by a client: the node proves the file it resolves to sits
under the session tree of the transcript *this node* derived for *this pane*, which is a fact the
request has no say in. Everything else is `not_found`, in the one wording every refusal here uses.

### `error`
```jsonc
{ "t": "error", "code": "not_writer", "message": "this device is read-only", "pane": null }
```
Codes: `not_writer` · `unknown_pane` · `node_offline` · `herdr_unavailable` · `bad_request` ·
`not_found` · `revoked` · `unsupported` (the node does not implement that op) ·
`stream_unavailable` (the herd is reachable and this pane's *screen* is not).

There is no `rate_limited` error code. Pairing throttling is an HTTP **429**, so nothing ever
emitted one and a client written to expect it was written against a code that does not exist.
Relayed errors are the one exception to this list being closed: a hub forwards a peer's `code`
verbatim, so a newer peer's code reaches a client unchanged rather than being dropped.

`herdr_unavailable` and `node_offline` are both emitted for a herd outage: the first names the
cause on the connection, the second says which node went with it, and an op addressed at a pane on
a node that is down is refused with `node_offline` rather than left to time out. **Both carry
`node`**, the id of the node they are about:

```jsonc
{ "t": "error", "code": "node_offline", "message": "workbox is offline", "node": "01JWORKBOX" }
```

Additive and optional — absent is what every error carried before it, and absent is what a
pane-scoped or connection-scoped error carries still, so an installed client that has never heard
of the field behaves exactly as it did. It exists because **only the client can tell a fault from
an interruption**: a node going unreachable used to arrive with no subject at all, so a client
showed it the only way it could, as a strip over whatever screen was open — and a node the
operator was not looking at interrupted a pane on a different one. The node cannot know which
pane is on screen and must not guess. A client that reads `node` should say it loudly only when
it is about the node whose pane is in front of the operator, and otherwise leave it to the herd
screen, where `nodes[].online` and `nodes[].detail` already say the same thing quietly.

`stream_unavailable` is **not** an outage and is deliberately its own code: the node is answering,
the pane list is right, and only the frames are missing. It always names a `pane`, it carries the
same words as that pane's `herd` `detail`, and it is sent **once per fault**, on the edge — a
supervisor retrying for ever must not raise a strip for ever. Recovery has no frame of its own:
the `herd` entry clearing is what takes the notice down, and `error` has no form that means
*never mind*. A client that only knows the v1 code list shows `message`, which is the rule this
list already runs on, so a 0.1.9 phone gets the diagnosis and the fix in the error strip it
already draws.

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

{ "t": "resync" }                       // node replies with herd + grid.reset for every watched pane
{ "t": "ping", "n": 7 }                 // -> {"t":"pong","n":7}
```

There is **no resize message on the client socket**, and there will not be one: a resize is not
something a viewing client does. Reshaping a pane is a `manage` op — `pane.size` — asked for
explicitly, like every other structural action ([ADR 0012](adr/0012-one-deliberate-resize-behind-a-panel.md)).

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
- **`grid.reset` is sent once the stream exists, never before it.** The geometry is a promise that
  rows are coming; a pane the node cannot stream gets `error{stream_unavailable}` and a `detail` in
  the herd instead, and no grid at all until it can keep the promise.
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
  to fill with, so fill-height wins and the surface pans horizontally. `scrollback_rows == 0` says
  only that there is no history *above* the grid — the live grid comes off the observe stream and is
  unaffected — so it is not a reason to show a pane on any surface other than the one this device
  last chose for it. A pane opens on its remembered `view`, and on the terminal when it has none.
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

// The one op that reshapes a pane's PTY, and the only caller of `terminal session control` in the
// product (ADR 0012). `mode` defaults to "once": claim, resize, release, then *measure* — on a pane
// with a desk attached the geometry is restored the moment the claim goes (#19), and the ack says
// `kept` and `measured_rows` rather than echoing what was asked for. "hold" keeps the claim so the
// size survives there, at the cost of that desk rendering wrong while held (#298); "release" ends
// one. Refused below 80x24, because a headless resize persists (#219, #305) and a pane fitted to a
// small screen would stay that narrow for everybody.
{ "t": "manage", "op": "pane.size",        "at": "01J.../w3:p2", "cols": 200, "rows": 50 }
{ "t": "manage", "op": "pane.size",        "at": "01J.../w3:p2", "cols": 200, "rows": 50, "mode": "hold" }
{ "t": "manage", "op": "pane.size",        "at": "01J.../w3:p2", "mode": "release" }
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

A `session.create` or `session.stop` ack is a promise that the **host already agrees**: the node
waits for `herdr session list` to show the change, and reconciles the herd, before acking. Both ops
finish before the state they changed is visible — `server.stop` answers `ok` 52 to 303 ms before the
session stops being listed as running (#241) — so a client refreshing `caps` on its own ack used to
be handed back the state it was trying to change. It may now refresh on the ack and trust the answer,
and the session's node has appeared in, or gone from, the herd by the time the ack lands.

**A `manage` op on a node whose herdr is stopped starts it.** Every other op needs the socket, so
one that finds it dead asks herdr which session owns this node's socket — the list carries it
whether or not anything is running (#326) — starts that server, waits for it to *answer a call*
rather than merely accept a connection, and then does what it was sent to do. `default` is a
session name like any other for this purpose: `herdr server --session default` binds the default
socket rather than making a namesake beside it (#324). Racing is safe rather than negotiated: a
second server for a session already running exits 1 and changes nothing (#243, #325), so two
clients tapping at once cost one dead child process.

Nothing else in a node ever starts herdr. Watching, polling, reconnecting and the herd sweep all
find the same stopped server and leave it stopped — an operator who is not using herdr on a host is
not asking for one. This is what makes a rarely-visited machine usable from a phone: the node
answers, the herd is one offline node with no panes, and the first `workspace.create` brings the
machine up. A client should therefore keep offering `node`-scoped ops on a **local** node that is
offline. A **peer** that is offline is a different question — the mesh link may be what is down,
and nothing in the herd tells the two apart.

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
correct for an explicit action and does not conflict with "looking at a pane never reshapes it" — that
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
  "build": "0.1.21", "key": "<hex ed25519 public key>", "nonce": "<hex 32 bytes>",
  "join": "K7QF-9M2X" }        // only on the connection that enrols this node; omitted after
// hub → peer
{ "t": "mesh.challenge", "protocol": 1, "node_id": "01J...", "node_name": "front",
  "build": "0.1.21", "key": "<hex>", "nonce": "<hex 32 bytes>" }
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
`wrong_hub` · `wrong_node` · `protocol`.

**A node id is bound to the key that enrolled with it.** `node_id` and `node_name` are the peer's
own words about itself — the handshake authenticates a *key* and proves nothing about either — and
the hub routes watches, keystrokes and manage traffic on the id. So the enrolment row records what
the connection that spent the join code claimed, an ordinary reconnect writes neither back, and a
`hello` is refused `wrong_node` when its `node_id` differs from the enrolled row's, when another
enrolled node holds it, or when it is the hub's own. A node that changes its id has to be revoked
and given a fresh code, exactly as one that regenerates its key does. Both fields are bounded and
checked before the challenge: an id is 1–64 characters of `[A-Za-z0-9._-]` — never a `/`, which
separates a node from its pane — and a name is 1–64 characters with no control characters.

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
- **What `--fingerprint` protects, and what it does not.** It protects this node's *signature*:
  the pin is checked against `mesh.challenge` before `mesh.auth` is sent, so a stranger answering
  at the hub's address never collects one. It does **not** protect the join code, which travels in
  `mesh.hello` — the first message on the socket, sent before any challenge can arrive — so an
  impostor answering at that address has harvested a live single-use code by the time the
  fingerprint is compared. The message order is deliberate and is not changing: what covers the
  code is TLS, so join over `wss://`, and treat a `wrong_hub` refusal as a code to re-mint rather
  than as a typo.
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
