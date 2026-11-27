# Muffit sessions

[muffit](../../muffit) is a coding agent with its own daemon. It has no terminal: a session is a
structured conversation (messages, tool calls, permission prompts, workers) served over REST and a
websocket. Its contract is `../muffit/docs/api/kampr.md` (wire 3.3, read up to 3.9) with the frame types in
`../muffit/docs/api/events.schema.json`. This brief is how Kampr shows one.

## The shape

**A node talks to the muffit daemon on its own machine, and the mesh does the rest.** The node
lists that daemon's live sessions as panes of its own herd. A hub relays them like any other peer
pane, because `convo`, `convo.turn`, `pending` and `convo.facets` already pass through the relay by
pane id and `input`/`answer` are relayed back. Muffit's own hub (`muffit.oldug.com`) is never needed:
its `remote` rows would show every session twice.

- **Reaching it.** `[muffit]` in the node's config: `url` (default: `[server] host` and `port` from muffit's
  own config, found as muffit finds it — `$MUFFIT_CONFIG_FILE`, else `$XDG_CONFIG_HOME/muffit/config.toml`,
  else `~/.config/muffit/config.toml` — and `http://127.0.0.1:7891` when that says nothing),
  `token` or `token_file` (default `$MUFFIT_SERVER_TOKEN`, else `$MUFFIT_HOME/server.token`, else
  `~/.local/share/muffit/server.token`), `enabled` (default: on when a token is found). Every
  request sends `Authorization: Bearer` and **no `Origin` header**: muffit refuses browser-style
  requests with a 403 (kampr.md §1), which is also why a browser cannot reach it without the node.
- **A muffit daemon that is down is a supported state.** The provider lists nothing and retries;
  it never fails the herd and never marks the node offline.
- **A daemon that answers and refuses the token is not.** Two daemons can share a host (a container's
  holds 7891 while the operator's listens on 7892, as on giftofthemagi2), and the node's token opens only
  one of them. A 401 or 403 on the session stream is a `WARN` naming the url and status, once per change
  of state rather than once per retry, and `kampr doctor`'s `muffit` check fails on it with the line to
  set. The node's start-up line says where the url came from (`from=`).

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

**A finished conversation is not a pane.** muffit lists a session as live only while a turn runs
or a client holds it (the TUI, the web UI), or until something that started it with no client
stops it (a daggit wake, `POST /sessions`). A session leaves the herd when its turn ends with
nobody holding it, as a claude that exits leaves its pane. Showing muffit's stored sessions would
be listing every old claude conversation.

**Kampr only ever watches.** Its attach is `?observe=true` (kampr.md §3): it never restores a
stored session and never holds a live one, so opening and closing somebody else's session cannot
let it go and end its background jobs. A pane that leaves the list while somebody watches it is
read over REST. Typing into one that has left sends the message over REST, which brings it back.

**A terminal running the TUI is the session's pane.** muffit lists each session's attached
`clients` with the pid and hostname each named in its hello. A `tui` client whose pid is one of a
terminal pane's processes, on this machine, makes that terminal the session's pane: `agent`
`muffit`, the session's title and status, its conversation, prompts and answers from muffit as
below, and no `m:` pane beside it. Typing stays the terminal's, so the reply box types into the
TUI as it types into claude. The `m:` panes left are the sessions no terminal is showing, which
the client lists under its own heading.

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

`answer {key}` on a muffit pane goes to its open conversation, or straight to the daemon when
nobody is watching it, and posts `allow`/`always`/`deny` to `POST /sessions/{id}/permission` with
the prompt's id; it is never keystrokes. A 409 (answered
elsewhere) clears the strip. The prompt reaches the herd too, so push names it the way it names a
screen-read question.

## What a newer daemon adds

The attach's `hello` names the daemon's wire version, and the pane declares only what that version
serves (`kampr_muffit::abilities`): a 3.3 daemon gets exactly the pane described above. The node
greets as 3.9 and always asks for `events`.

| Since | The daemon gives | The pane gets |
|---|---|---|
| 3.4 | `question` on the snapshot and session list | A `pending` with id `q<n>`, an option per answer and `Type something` (`text: true`) when `free_text`. An answer goes as the option's index, or the words, to `POST /sessions/{id}/question`; a `reply` beside the key is ignored, since every client sends the permission shape. The pane is `blocked` while one waits |
| 3.5 | A worker's `attach?observe=true` | Opened beside `convo.sub`; each event it pushes re-reads the worker's transcript at once. The 1.5 s poll stays as the floor |
| 3.6 | `shell_tasks`, `job.stop` | `convo.tasks` rows `kind: "shell"`, titled by the command; `stoptask` sends `job.stop` with the id as a string (`"job1"`, whatever kampr.md's example shows). Their output is not on the wire |
| 3.7 | `tool_call_delta`; `rename`, `clear`, `queue.unqueue`, `fork` | The call being typed in `convo.live`; the verbs as steers and actions |
| 3.8 | `commands.list` | The session sheet's actions (controls, answers and `commands.list` itself left out; arguments from `handle_control`), `/` completion, and a `/<action> words` line run as that verb instead of being sent to the model. Before 3.8 the seven verbs every 3.x takes |
| 3.9 | `billed_to`, `rate_limit` | `convo.session` billing (the catalog name) and `rate_limited`; `convo.limits` says `rate-limited` and when it lifts |
| 3.10 | `PromptRaised` / `PromptSettled` events | The `pending` raised and cleared without waiting for the next status push. The daemon that records them still greets as 309, so this is read whatever the version says |

`context` carries the strategy's budget (`pressure.budget`) as the total; with none the facts line
says the size alone. Pause, resume and stop are REST verbs only: the socket's `handle_control`
answers them `unknown verb`.

## Input

The composer sends the text and then `"\r"`. On a muffit pane the node gathers the text per socket
and posts it as one `input` when the `"\r"` arrives, with a `msg_id` so a resend after a reconnect
runs once. `ctrl-c` and `esc` interrupt the running turn. Everything else a terminal would take
(arrows, bracketed paste framing) is dropped.

**Pictures go with the message.** A paste is written to the node's pastes directory and its path
typed, as on any pane. When the line is sent, each word that is one of those pasted images (png,
jpg, gif, webp) goes as an `images` entry instead, and leaves `[image N: <file>]` where it stood
in the text — the TUI's own label — so words around it still point at it (#621); muffit shrinks
the picture to what the model takes. The reply box holds such a file as a label at its caret and
puts the path there when the reply goes. In a terminal running the TUI the path is typed as a bracketed paste, which the
TUI attaches itself.

## Making and ending one

The + sheet has a **Muffit session** card: a directory, a first prompt, and **In a terminal** (on
by default), sent as the manage op `muffit.create {cwd, prompt, terminal}` → `POST /sessions`. In
a terminal, the node then starts a native session running `muffit --resume <id>` through the login
shell (`[muffit] tui` names another command) and acks that pane; it becomes the session's pane once
the TUI attaches. Without one, the prompt is required and the ack is the session's `m:` pane.
Closing a muffit pane is `POST /sessions/{id}/stop`.

## The client

A pane whose `provider` is `muffit` is a conversation and nothing else: it opens on the
conversation view on every breakpoint, has no terminal switch, no zoom/size controls, and is never
counted as a terminal that failed to paint. Its row in the herd is marked as a muffit session, and
listed after the machine's terminals under **Background · muffit**. A terminal running the TUI is
an ordinary terminal pane with a conversation tab.

## What older builds do

Additive throughout. An older hub passes `provider: "muffit"` and the conversation frames through.
An older client sees an agent pane that converses, opens it on the conversation on a phone, and on
a desk opens a terminal that never paints, from which the conversation tab still works.
