# kampr-tui parity with the wire

The terminal client (`crates/kampr-tui`) against everything the wire carries, generated from
`kampr-core/src/wire.rs`, `kampr-core/tests/fixtures/wire-shapes.txt` and the node's manage ops
(2026-11-27). "Web" is the Compose client; "TUI before" is this crate before this pass. Decoding is
`kampr-client/src/convo.rs`; drawing is `kampr-tui/src/convo/declared.rs` (rows above the reply
box), `convo/sheet.rs` (session and tasks sheets), `headless.rs`, `resume.rs`, `program.rs`.
Tests: `kampr-tui/tests/conversation/{declared,headless,programs}.rs`, `tests/it/pointer.rs`.

## Server frames

| Frame / field | Web | TUI before | TUI now |
|---|---|---|---|
| `hello`, `role`, `herd`, `herd.patch`, `styles`, `grid.*`, `scrollback`, `find`, `error`, `pong` | yes | yes | unchanged |
| pane `modes` (docs/26) | wheel, click | wheel | wheel and click: an armed pane hears a click only under an SGR mouse mode, or with no modes reported. muffit is named by the `cmd` stem when herdr gives no `agent` (#699) |
| pane `program_error`, `program_msg` (docs/24) | Failed mark | no | `✗` on the sidebar row; "✗ failed: msg" after the pane's name in the strip and border |
| pane `program_progress` | hairline | no | `NN%` after the name, row and title |
| pane `program[]` | Programs sheet | no | "N programs" after the name; the pane menu's `programs` (`g`) lists them under their parents |
| `convo`, `convo.turn` (with `sub`), `convo.find`, `convo.facets`, `claude.hooks` | yes | yes | unchanged |
| `convo.composer` `text`, `clear` | yes | yes | unchanged |
| `convo.composer` `caret`, `keys` | mirror box | no | not built: the TUI's box is its own, never mirrored |
| `convo.heard` | ack a send | decoded, unused | drops this client's matching queued send |
| `pending` (every field, `kind`, `fields`) | card; a form's `1` opens its fields | strip; a form's `1` fills its fields in the box one at a time | both send `reply.form.values`; a form's fields are its context line. See [asks](#asks-and-the-node) |
| `pasted` | yes | no | not built: the TUI sends typed pastes only |
| `convo.abilities` | gates pieces | no | gates every piece below |
| `convo.live` | live turn | no | `✎` row: thinking estimate, last line, tool calls; empty withdraws it |
| `convo.tasks`, `convo.task` | activity bar, task sheet | no | `◐ N running` row; tasks sheet, output via `convo.follow` |
| `convo.state` | gates stop and queue | no | same gates |
| `convo.trouble` | banner | no | `⚠` row with retry and hold; `null` clears |
| `convo.session`, `.limits`, `.context` | facts line, session sheet | no | `≡` facts row where `account`/`billing`/`limits`/`context` is declared; the session sheet |
| `convo.controls`, `convo.actions` | session sheet | no | session sheet, the `confirm` sentence first |
| `convo.suggested` | chips | no | `tab` row for the current `/` or `@` word |
| `convo.acted` | sheet line | no | sheet line and status note |

## Client verbs

| Verb | Web | TUI before | TUI now |
|---|---|---|---|
| `watch`, `unwatch`, `find`, `done.read`, `resync`, `ping`, `paste` | yes | yes | unchanged |
| `input` | text | text then `\r` | to a source declaring `queuenext`, `sendnow` or `unqueue`: one `input` with a minted `id`, `when: next` while a turn runs, `now` for send-now; otherwise unchanged |
| `input.unqueue` | no | no | `alt+u`: the newest send the harness has not heard, its words back in the box |
| `answer`, `answer.submit` | yes, `reply` allow/deny only | yes, no `reply` | unchanged (the node maps 1/2/3 itself) |
| `convo.load`, `convo.sub`, `convo.find` | yes | yes | `convo.sub` also from the tasks sheet |
| `convo.steer` | interrupt only | no | `alt+k` interrupt; every other declared steer from the session sheet (`rename`, `aside` ask for words); `stoptask` from the tasks sheet |
| `convo.set`, `convo.act` | yes | no | session sheet |
| `convo.follow` | yes | no | tasks sheet |
| `convo.suggest` | yes | no | as the box changes: `/` with `slash`, `@` with `mention` |

## Manage ops

| Op | Web | TUI before | TUI now |
|---|---|---|---|
| structure, `focus`, `pane.*`, `layout.*`, `session.*`, `native.create/desk/forget`, `docker.*`, `fleet.run`, `claude.hooks.*` | yes | yes | unchanged |
| `claude.resumable`, `claude.create`, `claude.resume` | New sheet | no | host menu `c` on a node with `claude` |
| `claude.to_terminal`, `claude.end`, `claude.from_terminal` | pane actions; `busy` then confirm then `interrupt` | no | pane menu (right-click: `t`, `e`, `h`; the manage menu without keys), same `busy` flow; a made pane is opened on its ack |
| `native.resumable`, `native.resume`, `native.resume.dismiss` | Resume after reboot group | no | asked of each serving node with `native`, once per socket and again when it rejoins; listed under the node on the herd screen and in its host menu (`enter` resume, `x` dismiss) |
| `native.close`, `muffit.create`, `fleet.stop/forget/save/drop` | yes | no | not in this pass |

## Asks and the node

`pending.kind` says what answering means (`permission`, `question`, `plan`, `form`) and a form
carries `fields`. A key press is the node's to read against the ask it holds: a headless Claude
pane takes it as `Command::Press` (a question's key chooses or ticks, its last option takes words,
a plan's `3` revises with the words) and `answer.submit` as `Command::Submit`; a hooked Claude
and muffit read keys the same way. Only a form needs a `reply`: its `1` is sent with
`reply.form.values`. The TUI fills the fields in the box, one per Enter, typed as each field says
(`integer`, `number`, `boolean` as y/n, a `choices` field one of them; an empty optional field is
left out). A hooked form has no `fields` and stays the terminal's (docs/12).

| Ask | Headless Claude | Hooked Claude | muffit |
|---|---|---|---|
| question: option, ticks + submit, words | yes | yes | option and words; no multi |
| plan: approve, revise with words | yes | yes | none asked |
| form with values | yes | at the terminal | none asked |

## Keys

In a conversation view. Each is answered only where its gate holds; otherwise the key is the box's.

| Key | Does | Gate |
|---|---|---|
| `enter` | send; queues behind a running turn | queues under `queuenext` |
| `alt+n` | send now | a turn runs and `sendnow` |
| `alt+u` | take back the newest unheard send | `unqueue` |
| `alt+k` | stop the turn | a turn runs and `interrupt` |
| `alt+s` | session sheet: facts, controls, actions, steers | any of those declared |
| `alt+t` | tasks sheet; `enter` opens output or a subagent, `x` stops one | tasks listed; `x` needs `stoptask` |
| `tab` | take the first suggestion | suggestions for the current `/` or `@` word |

In a sheet: `↑`/`↓` move, `enter` changes or runs, `esc` backs out or closes, `y`/`n` answer a
confirm sentence. Host menu (navigate mode `.`): `c` Claude without a terminal (directory,
`enter`, then a first prompt and `enter`, or `↓` to a stored conversation and `enter`), `↑`/`↓`
over the records to resume, `enter` resume, `x` dismiss.
