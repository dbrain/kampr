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

## Server → client

### `hello` — first message on every connection
```jsonc
{ "t": "hello", "protocol": 1, "node_id": "01J...", "node_name": "comingclean",
  "build": "0.1.0+abc1234", "role": "full",          // "full" | "readonly"
  "caps": { "push": true, "scrollback": true, "conversation": true } }
```

### `herd` — the whole model; sent after `hello` and on any reconnect
```jsonc
{ "t": "herd",
  "nodes": [ { "id": "01J...", "name": "comingclean", "kind": "local",   // "local"|"peer"
               "online": true, "rtt_ms": 0.4, "herdr_version": "0.8.2" } ],
  "panes": [ { "id": "01J.../w3:p2", "node_id": "01J...",
               "workspace": "kampr", "tab": "1", "cwd": "/home/dbrain/dev/kampr",
               "label": null,
               "agent": "claude",                    // null on a shell pane — picks the default view
               "agent_status": "blocked",            // idle|working|blocked|done|unknown
               "cols": 74, "rows": 30,               // native, from the layout rect
               "scrollback_rows": 0,                 // 0 = no ring (alt screen) or unsafe to read
               "has_conversation": true,             // a journal adapter exists for this harness
               "updated_at": "2026-08-20T13:44:02Z" } ] }   // stamped by the node; Herdr's snapshot carries no time
```

`herd.patch` carries the same shapes under `added` / `changed` / `removed_ids`.

### `styles` — append-only style table, per connection
```jsonc
{ "t": "styles", "from": 12,
  "styles": [ { "fg": {"k":"r","v":[255,120,0]}, "bg": {"k":"d"},
                "bold": true, "underline": true } ] }
```
`fg`/`bg` are `{"k":"d"}` default, `{"k":"i","v":n}` indexed 0–255, or `{"k":"r","v":[r,g,b]}`.
Boolean attributes are omitted when false: `bold dim italic underline blink reverse strike hidden`.
Style `0` is always the default pen. Ids are stable for the life of the connection.

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

**`links` is a delta, and it may appear on `grid.patch`.** A hyperlink can first be seen mid-stream,
so a client that only reads `links` from `grid.reset` will render link ids it was never given.
Append each message's `links` to the pane's table in arrival order; ids are indices into it.

Measured compression against per-cell JSON: **61×** at 124×50 with 49 distinct pens (4,205 vs
257,985 bytes) and **44×** at 74×30 with light colour (1,769 vs 78,471). The ratio holds; the
absolute size scales with how much colour is on screen.

### `scrollback` — history above the viewport, oldest first
```jsonc
{ "t": "scrollback", "pane": "01J.../w3:p2", "from_top": 0,
  "rows": [ /* RowDiff, row = absolute index from the top of the node's ring */ ],
  "total_rows": 171, "complete": true, "capped": false }
```
Only sent for panes with `scrollback_rows > 0`. Sourced from `pane.read recent format=ansi` and run
through the same emulator, so styling matches the live grid. Agent panes never have this — their
history is the conversation.

**There is no `scrollback.load`, deliberately.** Herdr caps a scrollback read at **1000 lines** and
`pane.read` has no offset parameter (probe #51), so a client cannot page further back and neither can
the node — asking again just returns the same newest 1000. What the node *can* do is accumulate: while
it is watching, successive reads overlap, so it stitches them into a ring that grows past the cap.
History that scrolled away before the node started watching is unreachable, and `capped: true` says
so rather than pretending the top of the ring is the top of history.

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

### `pending` — a prompt is waiting
```jsonc
{ "t": "pending", "pane": "01J.../w3:p2", "question": "Do you want to make this edit?",
  "options": [ { "key": "1", "label": "Yes" }, { "key": "2", "label": "Yes, and don't ask again" } ],
  "source": "transcript" }              // "transcript" | "screen"
```
`source` records where the question came from. If a pending tool request turns out not to reach the
transcript before approval (probe #40, still open), the node falls back to parsing `pane.read visible`
and sets `"screen"`. **Clients must not care which.**

### `error`
```jsonc
{ "t": "error", "code": "not_writer", "message": "this device is read-only", "pane": null }
```
Codes: `not_writer` · `unknown_pane` · `node_offline` · `herdr_unavailable` · `rate_limited` ·
`bad_request`.

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
{ "t": "convo.load",  "pane": "01J.../w3:p2", "before": "opaque" }
{ "t": "resync" }                       // node replies with herd + grid.reset for every watched pane
{ "t": "ping", "n": 7 }                 // -> {"t":"pong","n":7}
```

There is **no resize message and there will not be one.** The node cannot reshape a pane.

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

## Herd management

Everything you would do at the keyboard. Additive to v1 — a node that does not implement these
replies `error{code:"unsupported"}`, and a client hides what a node's `hello.caps` does not claim.

`hello.caps` gains `"manage": true` when the node exposes them.

```jsonc
// Structure. `at` is a pane, tab or workspace id depending on the verb.
{ "t": "manage", "op": "workspace.create", "node": "01J...", "label": "kampr", "cwd": "~/dev/kampr", "env": {} }
{ "t": "manage", "op": "tab.create",       "at": "01J.../w3",    "label": "tests", "cwd": "~/dev/kampr" }
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

// Named sessions are separate Herdr servers, so this one shells out on the node rather than
// calling a socket method. Same shape to the client; only the node knows the difference.
{ "t": "manage", "op": "session.create", "node": "01J...", "name": "agents" }
{ "t": "manage", "op": "session.stop",   "node": "01J...", "name": "agents" }
```

Every `manage` op is acknowledged with `{"t":"managed","op":…,"ok":true,"id":"<new id, when one was created>"}`
and the resulting structure change arrives as an ordinary `herd.patch`. Clients must not
optimistically mutate their herd model — wait for the patch, so the node stays authoritative.

`readonly` devices are refused every `manage` op with `not_writer`.

### Capability discovery

```jsonc
{ "t": "caps", "node": "01J...",
  "agent_kinds": ["claude", "codex", "gemini", "…"],   // from server.agent_manifests, not hardcoded
  "sessions": [ { "name": "default", "running": true }, { "name": "agents", "running": false } ] }
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

## Auth

The WebSocket carries a device token in `Sec-WebSocket-Protocol` or a `kampr_session` cookie; the
node resolves it to a device and a role before `hello`. `readonly` devices get every server → client
message and are refused `input` / `answer` with `not_writer`. HTTP endpoints for enrolment
(`/auth/pair`, `/auth/webauthn/*`) are specified alongside the auth work, not here.
