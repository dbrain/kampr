# Muffit sessions

[muffit](../../muffit) is a coding agent with its own daemon. It has no terminal: a session is a
structured conversation (messages, tool calls, permission prompts, workers) served over REST and a
websocket. Its contract is `../muffit/docs/api/kampr.md` (wire 3.2) with the frame types in
`../muffit/docs/api/events.schema.json`. This brief is how Kampr shows one.

## The shape

**A node talks to the muffit daemon on its own machine, and the mesh does the rest.** The node
lists that daemon's live sessions as panes of its own herd. A hub relays them like any other peer
pane, because `convo`, `convo.turn`, `pending` and `convo.facets` already pass through the relay by
pane id and `input`/`answer` are relayed back. Muffit's own hub (`muffit.oldug.com`) is never needed:
its `remote` rows would show every session twice.

- **Reaching it.** `[muffit]` in the node's config: `url` (default `http://127.0.0.1:7891`),
  `token` or `token_file` (default `$MUFFIT_SERVER_TOKEN`, else `$MUFFIT_HOME/server.token`, else
  `~/.local/share/muffit/server.token`), `enabled` (default: on when a token is found). Every
  request sends `Authorization: Bearer` and **no `Origin` header**: muffit refuses browser-style
  requests with a 403 (kampr.md §1), which is also why a browser cannot reach it without the node.
- **A muffit daemon that is down is a supported state.** The provider lists nothing and retries;
  it never fails the herd and never marks the node offline.

## Panes

A live session is the pane `m:<session id>` with `provider: "muffit"`, `agent: "muffit"`,
`converses: true` and `has_conversation: true`. `label` is the session's title, `cwd` its cwd,
`cols` absent and `rows: 0`, since there is no grid. `stopped` sessions are left out. Workers
(`<parent>.s<n>`) are not panes; they are subconversations of their parent (below).

`agent_status` comes from the session's state (kampr.md §4) and its prompt:

| muffit | `agent_status` |
|---|---|
| `permission` set | `blocked` |
| `running`, `waiting_tool`, `waiting_provider` | `working` |
| `waiting_user`, `completed` | `done`, which reads `idle` after `done.read` exactly as other agents do |
| `created`, `paused` | `idle` |
| `failed` | `done` |

The node holds `GET /sessions/stream` open for the list, so status, titles and prompts reach the
herd (and so triage and push) without anybody watching the pane.

**A finished conversation is still a pane.** muffit lists a session as live only while a turn runs
or a client is attached, so the one that answered a minute ago is gone from `/sessions` and the
stream. The node also polls `GET /sessions?all=true` and keeps every stored session updated in the
last seven days (newest 20) as a pane, `idle`. Watching one reads its transcript over REST and never
attaches, because attaching restores the session (kampr.md §2): looking at a conversation must not
start one. Typing into it is what attaches, and the pane is live from then on.

**Prompts need a daemon that waits.** With nobody attached, muffit refuses a permission prompt at
once unless its `[server.prompts] wait_secs` is set (kampr.md §5). The node's session stream sees a
prompt that waits, which is what puts it in triage and push; it does not attach to every session to
be an answerer.

## The conversation

**Watching a muffit pane starts no terminal pump.** It starts a muffit pump: the attach websocket
(`hello` with `history`, `status`, `subscribe`, `permissions`), the transcript over REST, and
what the socket pushes after it.

- **Turns.** One `WireTranscriptEntry` is one turn, id `m<seq>`. `user` is a person's prompt and
  `assistant` the reply. `tool` entries are not turns: each `tool_result` is folded into the
  `tool` block of the call it answers (by `call_id`), which revises that turn. `system` entries
  and `recalled`/`notice` blocks are dropped. Blocks map as:
  - `text` → `md`
  - `tool_call` → `tool {name: tool, summary: <args, one line>, state}`, `state` running until
    its result arrives, `error` for `failed`, else `done`
  - `tool_result` → completes its call: `lines: total_lines`, and a `code {role: output}` block
    with its `summary`
  - `diff` → `diff {path, text}` (the hunks rendered as a unified diff)
  - `tool_call` of `delegate` → also a `sub {id: <worker id>, kind: "worker", title}` once the
    worker is known (`snapshot.subagents` by `call_id`)
  - `image` → `md` with the `[image · <mime>]` marker; `thinking` is dropped
- **Live.** `content` frames while a turn runs build the reserved `live` turn (text, tool calls,
  tool output). A `recorded {seq}` block means the transcript covers it: the node re-reads the
  transcript after its watermark, sends the new turns, and withdraws `live`.
- **Paging and search.** `convo.load` pages with `before_seq`. `convo.find` searches the
  transcript the node fetched.
- **Workers.** `convo.sub` with a worker id serves `GET /sessions/<worker>/transcript` and follows
  it while it is open. A worker can't be attached (kampr.md §6), so the follow is a poll.
- **Facets.** `title`, and `running` from `snapshot.subagents` whose liveness isn't `Dead`
  (`kind: "worker"`, `title`, `call`).

## Questions and answers

A permission prompt is a `pending` with `source: "transcript"`:

```jsonc
{ "t": "pending", "pane": "N/m:s1", "header": "bash",
  "question": "run `cargo install ripgrep` with write access outside the project",
  "options": [ { "key": "1", "label": "Allow" }, { "key": "2", "label": "Always allow" },
               { "key": "3", "label": "Deny" } ] }
```

`answer {key}` on a muffit pane posts `allow`/`always`/`deny` to
`POST /sessions/{id}/permission` with the prompt's id; it is never keystrokes. A 409 (answered
elsewhere) clears the strip. The prompt reaches the herd too, so push names it the way it names a
screen-read question.

## Input

The composer sends the text and then `"\r"`. On a muffit pane the node gathers the text per socket
and posts it as one `input` when the `"\r"` arrives, with a `msg_id` so a resend after a reconnect
runs once. `ctrl-c` and `esc` interrupt the running turn. Everything else a terminal would take
(arrows, bracketed paste framing) is dropped.

## Making and ending one

The + sheet has a **Muffit session** card: a directory and a first prompt, sent as the manage op
`muffit.create {cwd, prompt}` → `POST /sessions` → the new pane's id. Closing a muffit pane is
`POST /sessions/{id}/stop`.

## The client

A pane whose `provider` is `muffit` is a conversation and nothing else: it opens on the
conversation view on every breakpoint, has no terminal switch, no zoom/size controls, and is never
counted as a terminal that failed to paint. Its row in the herd is marked as a muffit session.

## What older builds do

Additive throughout. An older hub passes `provider: "muffit"` and the conversation frames through.
An older client sees an agent pane that converses, opens it on the conversation on a phone, and on
a desk opens a terminal that never paints, from which the conversation tab still works.
