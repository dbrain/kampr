# Conversation sources: implementation plan

One seam between the conversation pane and whatever agent is behind it, with muffit and Claude as
its first two implementers, and the pane grown until a terminal beside it is a preference rather
than a need.

The measurements this rests on are in [docs/20](./20-conversation-parity.md) and the probe log,
#644 to #661. Read [docs/04](./04-wire-protocol.md) first: every wire addition here is additive or
it does not ship.

## Goal

1. A `Conversation` trait in `kampr-core` that carries everything a rich pane shows and does.
2. A capability description on every source, so the pane draws exactly the controls that source
   takes and a new control never needs a new trait method.
3. muffit ported to it, and muffit itself widened until Kampr reaches everything muffit's own TUI
   does.
4. Claude twice: with no terminal, over stream-json, held by a keeper so a node restart loses
   nothing; and beside a terminal somebody started, through transcript, hooks and files.
5. Nothing in it can move a session onto paid API usage, or into paid overage, without the
   operator having said so.

Not in this plan: Codex and ACP agents. Phase 9 names them as the test of the trait and no more.

## What is already here

Kampr has the generic feed twice, in two shapes that do not know about each other.

- **`kampr_muffit::follow`** is a conversation with no terminal behind it. A `Follow` takes
  `Command`s (`Input`, `Older`, `Find`, `Sub`) and emits `Update`s (`Page`, `Turns`, `Older`,
  `Facets`, `Pending`, `Found`, `SubPage`, `SubTurns`, `Missing`, `Failed`). `kampr-node/src/muffit.rs`
  turns each `Update` into one wire message in 20 lines.
- **`kampr-node/src/convo.rs`** is the same job for a harness in a pane: 1,457 lines that poll a
  `Journal`, lift a preview off the screen, fold in the hooks and emit the same wire messages.
  `session.rs` then special-cases muffit to route its commands to the other shape.

The muffit shape is the trait. It was written for an agent that is only a conversation, which is
what a headless Claude is. This plan lifts it into `kampr-core`, widens it, and makes the pane pump
one more implementer instead of the thing everything else is an exception to.

## Principles

- **Typed where the pane has to understand, described where it only has to offer.** Turns, asks,
  task output and trouble are typed. Settings and actions are *described by the source* (a list of
  controls with their kinds and choices), so muffit's `thinking.set` and Claude's effort, fast mode
  or output style appear without the trait knowing their names. This is what "limit nothing" costs
  in design: one generic control surface, not forty methods.
- **An ability the source does not declare is a control the client does not draw.** No source
  pretends, and none is held to the weakest one's set.
- **The wire of a harness is read leniently.** Unknown line types are skipped, a missing field is
  an older version, features are detected from the harness's own capability list (#644).
- **Measure, then build.** Each phase names the probes it needs before code.
- **Parity is a list, not an intention.** `docs/22-parity.md` holds one row for every key, slash
  command, dialog and status the harness's own TUI has (Claude's and muffit's), and each row says
  where it is in the pane or why it is not. Phase 0 writes the rows, every phase after it fills
  some in, and phase 8 is not done while a row is blank. A Claude or muffit release that adds
  something adds a row.

## The seam

`kampr-core/src/conversation/`: `mod.rs` (traits), `command.rs`, `update.rs`, `abilities.rs`,
`ask.rs`, `control.rs`, `facts.rs`. Turn, block and facet types move here from `kampr-journal`,
which keeps the parsers.

```rust
#[async_trait]
pub trait Conversation: Send + 'static {
    async fn run(self: Box<Self>, commands: mpsc::Receiver<Command>, emit: Emit);
}

#[async_trait]
pub trait Source: Send + Sync + 'static {
    fn kind(&self) -> &'static str;
    async fn open(&self, pane: &str) -> Result<Box<dyn Conversation>, Refusal>;
    async fn create(&self, new: NewSession) -> Result<String, Refusal>;
    async fn resumable(&self, cwd: Option<&Path>) -> Vec<Stored>;
    async fn end(&self, pane: &str, how: Ending) -> Result<(), Refusal>;
}
```

`run` is muffit's `Follow::run` unchanged in shape. `Source` is what makes a session exist: new,
resumed from a stored one, ended, or handed to a terminal (`Ending::ToTerminal`).

### Abilities

Sent as `Update::Abilities` when a conversation opens and again whenever they change (a model
switch can change which effort levels exist). On the wire each set is a list of strings, so a
client skips a word it has never seen.

```rust
pub struct Abilities {
    pub input: BTreeSet<InputAbility>,   // Images, Files, Slash, Mention, QueueNext, SendNow, Unqueue
    pub steer: BTreeSet<Steer>,          // Interrupt, Continue, Pause, Resume, Stop, Background,
                                         // StopTask, Compact, Clear, Rewind, Fork, Rename, Aside
    pub asks: BTreeSet<AskAbility>,      // Permission, Always, EditInput, Question, FreeText,
                                         // Plan, Form, Url
    pub streams: BTreeSet<Stream>,       // Text, ToolArgs, Thinking, TaskOutput, SubLive, Hooks
    pub facts: BTreeSet<Fact>,           // Account, Billing, Limits, Context, Cost, Trouble, Mcp
    pub lifecycle: BTreeSet<Lifecycle>,  // Create, Resume, End, ToTerminal, FromTerminal
}
```

### Commands

```rust
pub enum Command {
    Input { text: String, msg_id: String, images: Vec<Image>, files: Vec<FileRef>, when: When },
    Unqueue { msg_id: String },
    Answer { ask: String, reply: Reply },
    Steer(Steer, Option<String>),
    Set { control: String, value: Value },
    Act { action: String, args: Value },
    Older { before: Option<String> },
    Find { query: String, cap: usize },
    Sub { id: String, before: Option<String> },
    Task { id: String, from: u64 },
    Unfollow { id: String },
    Suggest { kind: SuggestKind, query: String },
    Refresh(Fact),
}
```

- `When` is `Idle`, `Next` or `Now`: queued behind the turn, or steering it. Claude declares these
  in its own capability list (`interrupt_send_now_v1`, `interrupt_cancel_queued_v1`).
- `Reply` is `Allow { input, remember }`, `Deny { message }`, `Choose { answers }`,
  `Plan { approve, mode, feedback }`, `Form { values }`, `Dismiss`.
- `Steer`'s argument is the task for `StopTask`, the message for `Rewind`, the title for `Rename`.
- `Suggest` is completion while typing: files for `@`, commands for `/`.

### Updates

```rust
pub enum Update {
    Abilities(Abilities),
    Page(Page), Turns(Vec<Turn>), Older(Page),
    Live(Live),
    Facets(Facets),
    Asks(Vec<Ask>),
    Found { query: String, found: ConvoFound },
    SubPage { id: String, page: Page },
    SubTurns { id: String, turns: Vec<Turn> },
    SubLive { id: String, live: Live },
    Tasks(Vec<Task>),
    TaskOutput { id: String, at: u64, text: String, ended: Option<TaskEnd> },
    State(RunState),
    Trouble(Option<Trouble>),
    Session(SessionFacts),
    Controls(Vec<Control>),
    Actions(Vec<Action>),
    Limits(Limits), Context(ContextUse), Cost(Cost),
    Suggested { kind: SuggestKind, query: String, items: Vec<Suggestion> },
    Acted { action: String, ok: bool, text: Option<String> },
    Notice(Notice),
    Missing(String), Failed(String),
}
```

- `Live` is the message being written: text so far, thinking (a token estimate where the text is
  withheld, #644), and tool calls with their arguments as they arrive.
- `Task` is a running or just-ended shell, subagent or monitor: `{id, kind, call, parent, title,
  since, backgrounded, state}`. It replaces `Facets.running`, which stays on the wire for old
  clients. `parent` is what makes a subagent's own shell a child row (#650).
- `Ask` is today's `Pending` plus `by` (which agent, #636) and `kind`. A list, because a
  subagent's request and the main agent's stand together (#657).
- `RunState` is `Idle`, `Running`, `Waiting`, `Paused`, `Ended`.
- `Trouble` is `{kind, text, status, retry: Option<{attempt, of, in_ms}>, fix: Option<Fix>}`.
  `Fix` is `Login`, `Wait { until }` or `Billing`.
- `SessionFacts` is `{id, cwd, title, model, mode, version, account, billing, started}`. `billing`
  is the subject of the safeguards below and is always present.
- `Control` is a described setting: `{id, label, kind: Choice | Toggle | Number | Text, value,
  options: [{value, label, detail}], group, confirm: Option<String>}`. Model, effort, thinking,
  permission mode, fast mode, output style and each MCP server's switch are all controls.
  `confirm` is the sentence shown before a change that costs money.
- `Action` is a described verb: `{id, label, detail, args: [ArgSpec], group, confirm}`. Adding
  something (a directory, an MCP server, a permission rule, a model the source can be given) is an
  action with described arguments. Every slash command a source lists is an action; so are
  compact, clear, rewind and fork where the source has them only as commands.

## The pane

Options are drawn as seven boards in the app's own `soft` tokens. Their sources are in
[`docs/design/conversation/`](./design/conversation/): one `.dc.html` a board and a `canvas.json`
that lays them out, the same format as the rest of `docs/design/`. They render on a Design canvas;
[this one](https://claude.ai/artifact/2uMZtffvMpNBRNNpuc1iyc) is a publish of those files and the
files are the copy that counts. Three aims decide between any two layouts: the transcript gets the
screen, anything shown can be copied in one step, and nothing a source offers is more than two
taps away.

| Piece | What it is | Replaces |
|---|---|---|
| **Header, one row** | Back flush with the edge, status mark, title on one line, the Terminal/Conversation switch, the session button. No second line | |
| **Sticky turn** | When a turn's own head has scrolled away, a 36 dp bar under the header names it (who, when, its first words) and carries its copy and collapse, as `StickyTurn` does today | |
| **Facts line** | One 28 dp line under the composer: model and effort, context size with how much of it was read from cache, the five-hour and weekly limits. Tapping it opens the session sheet | The model and limits being nowhere |
| **Transcript, no bubbles** | Full-width turns under a small speaker label that carries that turn's copy and its collapse. The operator's turns have a rule down their left edge in the accent colour, so the two voices are told apart at a glance and while scrolling. A run of tool calls is one bordered list, a row a call: status mark, tool, argument, the one number that matters (lines, `+14 −3`, elapsed). A running shell shows its last two lines under its row. Finished runs fold to a count | Cards that cost three lines a call |
| **Activity bar** | One 40 dp row above the composer while anything runs: what is running, how long, how many more. It opens the tasks sheet. Drawn as chips instead where two or three tasks are the usual case (a tweak on board 1) | `WorkingStrip` and `RunningStrip` as two stacked strips |
| **Tasks sheet** | Every running shell, agent and monitor as a tree, a subagent's shell under its agent. Selecting one shows its output or its conversation live, with copy all, background, stop, find, and "go to its call" | The strip being a read with no way in |
| **Ask dock** | The standing request with its full command (copyable), allow, deny, always, and beneath them deny-with-a-reason, edit the input, answer at the terminal. Says "1 of 2" and names the next and whose it is | `PendingStrip`, one ask at a time |
| **Trouble banner** | Under the header, one line and one button: retrying with attempt and countdown, logged out, limit reached and held. The header mark turns with it | Nothing |
| **Session sheet** | Billing and limits first; a search box over everything below; the source's controls as rows (choice, toggle, segmented); its actions as buttons; its commands as a list. The same search is `/` in the composer | Typing commands blind |
| **Composer** | Reply box, send. While a turn runs the button queues and a second sends now. A row of `/`, `@`, attach, the mode as a tappable word, and stop | Stop and mode living in the terminal |
| **Copy** | A copy control on every turn, every code block, every tool command and tool output. Long-press a turn to select; extend to more turns, to the end, or all; then copy as written, as Markdown, code only, or with tool output, share, or quote into the reply | One glyph per block and no way to take a span |
| **Wide screens** | The transcript keeps a readable measure and the tasks, session, files and search panels sit in a column beside it, so a subagent or a shell is read next to the turn that started it. Header facts become buttons | The phone layout stretched |

**This replaces the current conversation pane, in place, for every source.** There is no second
pane and no setting to choose between layouts. What keeps one layout right for a headless Claude
and for a harness Kampr can only read is that every piece is drawn from what the source declares,
and a piece with nothing to show takes no room:

| Piece | Drawn when | Otherwise |
|---|---|---|
| Facts line | The source declares any of `Account`, `Limits`, `Context` | Absent; the composer sits on the screen's edge |
| `/`, `@`, attach | `Slash`, `Mention`, `Images` or `Files`, each on its own | The control row is absent when none is declared and nothing else is in it |
| Mode word, Stop | A mode control exists; `Interrupt` | Absent. A terminal source that interrupts by a key still declares `Interrupt` |
| Queue and send now | `QueueNext`, `SendNow` | One send button |
| Activity bar | Something is running | Absent. With no `Tasks` ability it is one line naming what the agent is doing, not a button |
| Live tail under a shell row | `TaskOutput` | The row alone |
| Session button | The source lists any control or action | The pane's own actions only (search, zoom, files) |
| Ask dock's second row | `EditInput`, a reason on deny, a terminal to answer at | The three buttons |
| Trouble banner | The source reports trouble | Absent |
| Per-turn copy, collapse, the sticky turn, the operator's rule, selection | Always | |

Board 7 is the floor: a harness that declares turns and nothing else. It is the same pane with
fewer rows, not a different one. **Done when**, for the phase that ships each piece: the pane is
looked at on a phone with a headless Claude, a terminal Claude, muffit and the least capable
harness in a pane, and none has an empty row, a disabled control or a gap where one would be.

Each piece lands with the phase that gives it data: the transcript rows, copy and composer changes
need nothing new and can ship first; the activity bar, tasks sheet, ask dock, trouble banner and
session sheet arrive with phase 2 and fill out as sources declare more. `kampr-tui` gets the same
pieces in its own idiom.

## What each source fills

"Keys" means typed into the pane as today; "No" means the ability is not declared.

### Reading

| | Claude, no terminal | Claude in a terminal | muffit today | muffit, to add |
|---|---|---|---|---|
| Turns, older, search | `assistant`, `user`; transcript for older and search | Transcript | `content`, `history`, REST transcript | |
| Text as written | `text_delta` (#644) | `MessageDisplay` paragraphs (#623) | `content` every 16 ms | |
| Tool arguments as written | `input_json_delta` | No | `tool_call` `started` then `running` | Argument deltas |
| Thinking | `thinking_tokens`; `set_max_thinking_tokens.thinking_display` | Transcript | `thinking` blocks | |
| Tool output live | `get_task_output` polled, or the task file (#647) | Task file (#649) | `tool_output_chunk` | |
| Subagent conversation, live | `parent_tool_use_id` lines with `--forward-subagent-text` (#650) | `subagents/agent-<id>.jsonl`, `SubagentStart`/`Stop` (#656) | REST transcript of the child; progress lines on the parent | A child that can be watched live |
| Tasks, with parents | `task_started`, `task_updated`, `background_tasks_changed` | Transcript fold, `Stop.background_tasks`, `PreToolUse` | `snapshot.subagents`, tasks REST | Shell tasks in the snapshot |
| Diffs, images, files | Tool inputs and results | Transcript | `diff`, `image` | |
| Hook activity | `hook_started`, `hook_response` | No | n/a | |
| Compaction, notices | `compact_boundary`, `informational`, `notification` | Transcript, `PreCompact`/`PostCompact` | `notice` | |

### Asking

| | Claude, no terminal | Claude in a terminal | muffit today | muffit, to add |
|---|---|---|---|---|
| Permission, always, edit input | `can_use_tool` with `permission_suggestions` (#645) | `PermissionRequest` hook (#624, #625) | `PermissionPrompt` | Suggestions for "always"; edited input |
| Question, several answers, free text | `can_use_tool` on `AskUserQuestion` (#645) | Hook (#634) | No: its own gap 1 | A structured ask tool |
| Plan | `can_use_tool` on `ExitPlanMode`; probe | Hook (#635) | No | With the ask tool |
| MCP form, URL | `elicitation`; probe | Terminal only (#637) | No | |
| A subagent's ask, beside the main one | `can_use_tool` carrying the agent; probe | Hook with `agent_id` (#636, #657) | Local to the session: its gap 4 | Child and mesh prompts |

### Steering and settings

| | Claude, no terminal | Claude in a terminal | muffit today | muffit, to add |
|---|---|---|---|---|
| Send, images | `user` line; images probe | Keys, paste | `input` with `images` | |
| Queue, send now, unqueue | `priority`, `cancel_async_message`; probe | Keys | `input` steers a running turn | Unqueue |
| Interrupt | `interrupt` (#646) | Esc | `interrupt` | |
| Continue, pause, resume, stop | Stop is ending the process | Keys | `continue`, `pause`, `resume`, `stop` | |
| Background, stop a task | `background_tasks` (#648), `stop_task` | ctrl+b; keys | No | Both |
| Compact, clear | `/compact`, `/clear` as input; probe | Typed | `context.compact` | Clear |
| Rewind | `rewind_files` | Esc Esc | `revert`, `rewind`, `changesets` | |
| Fork, rename | `--fork-session`; `rename_session` | Typed | No | Both |
| An aside that stays out of the history | Probe | `/btw` typed | `btw` | |
| Model: list, switch, add | `list_models`, `set_model` (#646) | `/model` typed | `models.list`, `model.swap` | Adding a provider's model from a client |
| Effort, thinking | `set_max_thinking_tokens`, `apply_flag_settings` | Typed | `thinking.set` | |
| Permission mode, rules | `set_permission_mode` (#646), `list_permission_rules`, `update_settings` | shift+tab | `mode.set`, `permissions.set` | |
| Directories | `register_repo_root` | `/add-dir` | No | |
| MCP servers | `mcp_status`, `mcp_toggle`, `mcp_reconnect`, `mcp_set_servers` | `/mcp` | `mcp.*` | |
| Slash commands, with completion | `initialize.commands`; sent as input | Typed | No list | A command list |
| `@` file completion | `file_suggestions` | Keys | No | |

### Facts

| | Claude, no terminal | Claude in a terminal | muffit today | muffit, to add |
|---|---|---|---|---|
| Account and billing | `initialize.account` (#661) | A headless sibling asked (#658) | Provider and model in the session | Which key pays, per session |
| Limits | `rate_limit_event`, `get_usage` | Sibling (#658) | No | Provider rate-limit state |
| Context, cost | `get_context_usage`, `get_session_cost` | Transcript usage | `context`, `cost` | |
| Trouble and retries | `api_retry`, `assistant.error` (#651) | Transcript, `StopFailure` (#652) | `error` block, `turn` `failed` | Retry attempts as state |
| Run state | `session_state_changed`, `status` | Session file (#344), hooks | `state` | |

## Billing safeguards

The risks are specific, and each was measured or read off the binary's own help.

| Risk | How it happens |
|---|---|
| API billing instead of the plan | `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` in the environment takes precedence over the login, silently except for one stderr line (#661). An `apiKeyHelper` in settings does the same |
| A different provider | `CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`, or `ANTHROPIC_BASE_URL` |
| `--bare` | Reads no login at all: *"Anthropic auth is strictly ANTHROPIC_API_KEY or apiKeyHelper"*. Anthropic's documentation says it will become the default for `-p` |
| Paid overage | With usage credits enabled, a plan at its limit keeps going on credits. The stream says so: `rate_limit_event.isUsingOverage`, `overageStatus`, `get_usage.extra_usage` (#644, #646) |
| Turns nobody asked for | A finished background task wakes the model for a turn of its own (#647, #657); crons and loops do the same |
| Fast mode | Reported as `fast_mode_state`; how it is billed against a plan is **not verified here**, so it is treated as paid until it is |
| The gate or a probe spending the plan | A test that reaches the real API |
| Anthropic changing how `claude -p` is billed | Announced and paused once in 2026 |

The guards:

1. **A billing policy per node, default `plan`.** `[claude] billing = "plan" | "api" | "any"`.
   Under `plan` a session that would be paid any other way does not start.
2. **A scrubbed environment.** The headless child gets the operator's login environment, read once
   per source from `<login shell> -lc` the way a native session's terminal gets it, because a node
   run as a service has the service manager's `PATH` (`/usr/local/bin:/usr/bin` on .208) and the
   child's tools need `~/.local/bin`, cargo, nvm, proxies and certificates. Under `plan`,
   everything beginning `ANTHROPIC_` or `CLAUDE_CODE_USE_` and `CLAUDE_CODE_OAUTH_TOKEN` are
   removed from it; `api` and `any` keep them. `CLAUDE_CONFIG_DIR` crosses only when the operator
   configured it. The list lives in `kampr_core::child_env` beside the Codex one (probe row 697),
   and the keeper starts a pipe child from it alone.
3. **Asked before a word is sent.** The source sends `initialize` and reads `account` before the
   first prompt, which costs no turn (#658, #661). Under `plan` it requires a `subscriptionType`
   and no `apiKeySource`. Anything else ends the process and raises `Trouble { fix: Billing }`
   naming what was found: a key in the environment, no login, a third-party provider.
4. **Checked again on every turn.** Each turn's `system/init` carries `apiKeySource` (#644); a
   value other than `"none"` interrupts the turn and ends the session.
5. **Never `--bare`.** The launch line names what it wants, and guard 3 catches the day the
   default changes: a bare session reports `tokenSource: "none"` (#661).
6. **Overage is refused unless allowed.** `[claude] overage = "refuse" | "ask" | "allow"`, default
   `refuse`. On `isUsingOverage: true` the source interrupts, holds whatever was queued and raises
   `Trouble { fix: Billing }`. `ask` holds the prompt behind a confirmation instead. Limits are
   read before a prompt is sent, so a session at its limit is told so rather than started. While
   `ask` holds, only prompts wait: a word starting with `/` (`/compact` above all, how a spent
   window is made to last) goes to the child at once. `refuse`'s hold keeps everything.
7. **Turns the model starts are counted and capped.** A turn whose `result.origin` is a task
   notification or a schedule is marked self-started in the pane. `[claude] unattended_turns`,
   default 20 since the operator's last prompt, pauses the session when reached.
   `--max-budget-usd` is passed when `[claude] turn_budget_usd` is set, as a brake on a runaway;
   on a plan it counts list price, not money.
8. **Money-costing controls carry a confirmation.** Fast mode, and any model the source marks as
   outside the plan, are `Control`s with `confirm` set. A client cannot change one without showing
   the sentence.
9. **Billing is on the screen.** `SessionFacts.billing` is `{paid_by: Plan | ApiKey | Provider |
   Unknown, label, overage: bool}`. The pane's header details show it, and anything but `Plan` is
   a standing badge. muffit fills it from its provider.
10. **The side channel is cheap by construction.** The limits poll (#658) runs no turn; it is one
    process per login per minute at most, and only while a Claude pane is watched.
11. **No test reaches Anthropic.** Every harness that can start a Claude sets `ANTHROPIC_BASE_URL`
    to the mock endpoint, a throwaway `CLAUDE_CONFIG_DIR` and `billing = "any"`. The launcher
    refuses to start under `KAMPR_TEST=1` without a mock address, and a guard test fails on a
    harness that omits it. Live probes are scripts a person runs, on the smallest model, never
    part of `make check`.
12. **One login, one operator.** The source only ever runs the unmodified binary under the node's
    own user. It never relays another person's credentials, which is the line Anthropic's terms
    draw.

Each guard has a test against the mock endpoint: a key in the environment, a logged-out config, a
bare default, an `isUsingOverage` event mid-turn, the unattended cap, and the launcher under test
with no mock.

## Phases

Each ends with `make check` green and, where a person can see it, a release.

### Phase 0: probes

Appended to the probe log before the code that depends on them.

- Claude headless: plan approval; images and files on stdin; `priority` and
  `cancel_async_message`; slash commands as input (`/compact`, `/clear`, `/model`); `set_model`,
  `stop_task`, `rewind_files`, `rename_session`, `file_suggestions`, `mcp_*`, `elicitation`;
  `set_permission_mode` to and from `bypassPermissions` with
  `--allow-dangerously-skip-permissions`; whether `register_repo_root` is what `/add-dir` does; a
  subagent's `can_use_tool`; `result.origin` on each kind of self-started turn; how fast mode is
  billed, from Anthropic's documentation.
- Claude headless under a keeper: the keeper killed mid-turn; the node gone while a
  `can_use_tool` stands; stdin held open with nobody reading stdout.
- Claude in a terminal: a session made to fail (#652 was headless); two foreground shells in one
  batch and their task files (#649); a forked transcript through `ClaudeParser` (#655); the exit
  dialog's "Move to background and exit" (#660).
- Each run again outside a parent Claude, and the terminal ones on macOS.
- The parity list: `docs/22-parity.md`, from Claude's keybindings, its 68 commands, its dialogs
  and its status line, and the same for muffit's TUI against `docs/api/kampr.md`. muffit's half is
  the work list for phase 3.

### Phase 1: the seam, and muffit on it

**Owns** `kampr-core/src/conversation/`, `kampr-muffit`, `kampr-node/src/muffit.rs`, the muffit
paths in `session.rs`.

- The types above. `kampr-journal`'s turn, block and facet types re-exported from core.
- `MuffitSource` and `MuffitConversation` implementing the traits with today's behaviour, its
  abilities declared from what the daemon's greeting lists.
- One `conversations` registry in the node, keyed by pane, replacing the muffit special cases.
- No wire change. **Done when** the muffit integration tests pass unmodified and `session.rs` has
  no `kampr_muffit` path in it.

### Phase 2: the wire and the client learn the new vocabulary

**Owns** `kampr-core/src/wire.rs`, [docs/04](./04-wire-protocol.md), `client/shared`,
`client/conversation`.

- New frames, all additive: `convo.abilities`, `convo.live`, `convo.tasks`, `convo.task` (output),
  `convo.state`, `convo.trouble`, `convo.session`, `convo.controls`, `convo.actions`,
  `convo.limits`, `convo.context`, `convo.suggested`, `convo.acted`. Client to node: `convo.steer`,
  `convo.set`, `convo.act`, `convo.follow`, `convo.suggest`, and `answer` widened to `Reply`.
- Client: a controls sheet drawn from `Control`s; an actions palette from `Action`s, which is also
  `/` completion in the composer; a task panel that opens from a `RunningStrip` row onto a live
  subagent conversation or a live shell output; a trouble banner with its fix; billing and limits
  in the header details; the composer's send, queue and send-now. `kampr-tui` gets the same.
- muffit already fills several: `TaskOutput` from `tool_output_chunk`, controls from
  `models.list`, `thinking.set`, `mode.set`, actions from its control verbs.
- **Done when** a muffit session can have its model changed, its context compacted, a tool's
  output watched live and a worker's conversation opened, all from the pane.

### Phase 3: muffit widened

In the muffit repository, each a wire minor bump, tracked in its `docs/api/kampr.md` gaps list:

1. A structured ask tool (its gap 1), with several answers and free text.
2. A child session that can be watched live, and its prompts answered (gap 4).
3. Shell tasks in the status snapshot, with background and stop.
4. Tool-argument deltas; unqueue; clear; fork; rename.
5. A control and action listing verb, so Kampr reads muffit's settings and commands instead of
   hard-coding them. The test is its own TUI: anything a key does there appears in the listing.
6. Which provider and key a session is billed to, and provider rate-limit state.
7. Prompt events in the log (gap 3), typed REST bodies (gap 2).

### Phase 4: a keeper for a process that speaks lines

**Owns** `kampr-keep`.

- A pipe mode beside the pty mode: the child's stdin and stdout are pipes, both directions are
  appended to the log with a direction and a sequence number, and a client reads from a sequence.
- The control socket takes lines to write to the child.
- Lifecycle as [docs/17](./17-native-sessions.md): a `systemd-run --user` unit where there is one,
  detached otherwise; `exit.json` on the way out.
- **Done when** a node restart mid-turn, with a permission request standing, comes back to the
  same request and the rest of the turn, in a live test against the mock endpoint.

### Phase 5: Claude with no terminal

**Owns** a new `kampr-claude` crate; `kampr-node` wiring; the client's "new conversation".

- `wire.rs`: the stream-json reader and the control-request writer, with request ids and replies
  matched. Lenient by rule. Rust, not the TypeScript SDK: the node stays one binary with no
  runtime ([ARCHITECTURE §3](../ARCHITECTURE.md)).
- `launch.rs`: the argument line and the scrubbed environment; guards 1 to 5.
- `source.rs`: `ClaudeSource` (create, resume from the transcripts on disk, end, hand to a
  terminal) and a `Provider` so the herd lists these sessions as panes with no screen.
- `conversation.rs`: lines to `Update`s, `Command`s to lines. Controls from `initialize` and
  `list_models`; actions from `initialize.commands`; tasks from the task lines, output by polling
  `get_task_output` for followed tasks only; guards 6 to 9.
- History is the transcript through the existing `ClaudeParser`; the stream is the live edge.
- One opener: refuses a session id found in a live `sessions/<pid>.json` (#655).
- Bypass permissions is always on offer: every launch passes
  `--allow-dangerously-skip-permissions`, so the mode control lists it whatever mode the session
  started in, and `[claude] mode` sets the one it starts in. The mode is a word in the composer,
  one tap from every other mode.
- Hand-over: offered at rest; otherwise it names what stops, stops it, then starts `claude -r` in
  a native session, and the reverse (#659, #660).
- **Done when** a session is created from the phone, asks and is answered, has a shell watched
  live and a subagent opened, has its model changed, survives a node restart, is handed to a
  terminal and back, and every billing guard's test passes.

#### Starting a conversation without a terminal

- **Config.** `[claude] headless = true` is the default; `false` builds no source, so the herd's
  `nodes[].claude` is absent, the client offers nothing, and every `claude.*` op is refused with
  `headless Claude is off on this node ([claude] headless = false)`. The billing keys are the
  guards above: `billing` (default `plan`), `overage` (default `refuse`), `unattended_turns`,
  `turn_budget_usd`, and `mode`, `config_dir`, `binary` for how it starts (`binary` is resolved
  through the login shell when not set).
- **Phone.** New, Kampr side, *Claude (no terminal)*: a directory and an optional first prompt,
  sent as `claude.create {node, cwd, prompt?}`. The ack names the new `c:<keeper id>` pane and the
  client opens it; the first prompt is sent when that pane's conversation opens, after guard 3.
- **Resume.** The same step lists `claude.resumable {node, cwd?}`: the transcripts on disk for that
  directory, newest first, without any a live process holds. Picking one sends
  `claude.resume {node, id}`, which starts `claude -p -r <id>` where the conversation ran. A
  conversation open anywhere else is refused, because a second writer forks it (#655).
- **Hand-over.** A headless pane's actions offer *Open in terminal*: `claude.to_terminal {at, cols?, rows?,
  interrupt?}` stops the child, waits for it to exit, and starts `claude -r <id>` as a Kampr session
  in the same directory. A Kampr session running Claude offers *Continue without terminal*:
  `claude.from_terminal {at, interrupt?}` reads the session id from Claude's `sessions/<pid>.json`,
  refuses a session with no transcript yet (before its first prompt), hangs up that Claude
  (closing the session when Claude is the session itself) and resumes it headless.
  `claude.end {at}` ends a headless pane.
- **A busy Claude is asked about, an idle one is just moved.** Either hand-over of a Claude at rest
  goes on the first press, with no dialog. A busy one is refused with the code `busy` and a
  sentence naming what would stop: a headless child is read off its keeper's log (a prompt or a
  `system/init` with no `result` after it is the turn in progress; an unanswered permission,
  question, plan or form is named too), and a terminal Claude off its marker's status (`busy` the
  turn, `waiting` the question or permission, `shell` the `!` command; `idle` or no status at
  all, a Claude too old to write one, is at rest). The client shows that sentence as the
  confirmation, and on yes sends the same op with `interrupt: true`: the node sends a headless
  child an `interrupt` control request, or a terminal Claude Escape through its pane, waits up to
  5 s for it to come to rest, and moves it either way, since the hang-up stops whatever is left.
- **CLI.** The terminal client (`kampr-tui`) offers the `claude.*` ops; [docs/27](./27-tui-parity.md)
  lists them. A CLI that started a session itself, rather than asking the node, would not reach the
  node's herd until the node restarted.
- **What the guards refuse.** Under `plan`: a key, token or `apiKeyHelper` in reach, a
  third-party provider, or no login, before any word is sent; a turn whose `system/init` reports an
  `apiKeySource`; overage unless `overage` allows it; and self-started turns past
  `unattended_turns`. Each is a `Trouble { fix: Billing }` in the pane, never a silent fallback.

### Phase 6: Claude in a terminal, the gaps that need no trait

Can start any time after phase 0, and ships alone.

1. Trouble: `system/api_error` and `isApiErrorMessage` in `ClaudeParser`; `StopFailure.error` in
   `hooks.rs`.
2. Install `PreToolUse`, `SubagentStart`, `SubagentStop`; read `Stop.background_tasks`.
3. Shell output from the task files; subagent conversations followed live from the strip.
4. Limits, account and billing from a headless sibling per login (#658), behind guard 10.
5. Fork-aware transcript reading if phase 0 shows both branches drawn.

### Phase 7: the pane pump behind the trait

**Owns** `kampr-node/src/convo.rs`.

- `TerminalClaude` implementing `Conversation`: the pump's journal, preview and hooks become its
  inside. Steering it cannot do structurally is done by keys where a key does it (interrupt, mode,
  background) and not declared otherwise.
- The other harnesses in a pane (Codex, omp, pi, agy) get the same wrapper with a smaller ability
  set, which is what they have today.
- **Done when** the node has one path from a pane to a conversation, and the wire a client sees
  is unchanged for every existing harness.

### Phase 8: nothing left that needs the terminal

A pass over the parity list. Every row still blank is made a control or an action, or is written
down as needing a terminal and why: `/login`, the four
terminal-only commands (`doctor`, `color`, `focus`, `reload-plugins`), MCP forms beside a terminal.

### Phase 9: a third source

Codex's `app-server` or an ACP agent, probed first. Its only purpose here is to find what the
trait assumed about Claude and muffit.

## Order

```
0 ─┬─ 1 ── 2 ─┬─ 3 (muffit repository)
   │          ├─ 4 ── 5 ─┐
   └─ 6 ──────┴──────────┴─ 7 ── 8 ── 9
```

Phase 6 is the fastest relief for the sessions run today. Phases 1 and 2 are the investment
everything else spends. Phase 3 is independent of Claude entirely.

## Risks

- **The control protocol has no stability promise.** The reader is lenient and versioned by
  capability, and phase 6 is a complete fallback that uses none of it.
- **`claude -p` billing moves.** Guards 3 and 4 turn that from a surprise into a refusal with a
  reason.
- **Two writers on a session.** A terminal Claude and a headless one open together fork the
  transcript silently (#655). The one-opener rule covers Kampr's own sources; it cannot stop a
  person running `claude -r` by hand, so phase 6 item 5 has to hold regardless.
- **The described-control surface becomes a junk drawer.** The rule against it: a control or an
  action is only ever what the source itself listed. Kampr adds none of its own.
- **`claude --bg` grows a structured feed** (#654) and makes phase 4 redundant for Claude. The
  keeper's pipe mode is still what any other line-speaking agent without a daemon would use.
