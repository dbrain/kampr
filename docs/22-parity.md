# Parity: what the pane draws, what needs a terminal

One row for every key, slash command, dialog and status that a harness's own TUI has —
Claude's and muffit's — and for each, where the pane draws it (which piece, which phase) or
why it does not (a measured reason where it is terminal-only). This is how "nothing is
missing" gets checked: [Phase 8](./21-conversation-sources.md#phase-8-nothing-left-that-needs-the-terminal)
walks this list and turns every row still blank into a control or an action, or writes down
that a terminal is needed and why. The muffit half is the work list for Phase 3.

The pane pieces a row can land in are the ones in
[docs/21, "The pane"](./21-conversation-sources.md#the-pane): header, sticky turn, facts line,
transcript, activity bar, tasks sheet, ask dock, trouble banner, session sheet, composer, copy,
wide screens.

## How this was read

- **Claude, no terminal** and **Claude, in a terminal**: docs/21's "What each source fills",
  docs/20's measured findings, and the probe log, cited by row.
- **The command list**: a no-turn `initialize` request (the shape of #668) against Claude
  2.1.326 on the plan login in this project: **63 commands** (#672). The list is state-dependent:
  `login`, `add-dir`, `resume`, `btw`, `permissions`, `status` and `help` are not in this
  login's list. `login` is expected to appear when a session is logged out; the rest is
  undetermined. The plan estimated 68; this probe is the count that stands.
- **muffit today** and **muffit, to add**: `docs/api/kampr.md` (wire 3.3) with its gaps list,
  and [docs/18](./18-muffit.md). Phase 3 items are numbered as in docs/21.
- **Turns are blocked** (#670: the plan login refuses every prompt with a billing error).
  Cells marked "probe: blocked" are the Phase 0 items that need a turn; no-turn control
  requests keep working (#668, #669).

"Terminal-only" means the feature exists only in the TUI process; the last column says what
the pane does instead.

## Reading

| TUI feature | Claude, no terminal | Claude, in a terminal | muffit today | muffit, to add (Phase 3) | Where the pane draws it / why not |
|---|---|---|---|---|---|
| Turns, older, search | `assistant`, `user`; transcript for older and search | Transcript | `content`, `history`, REST transcript | — | Transcript, always drawn; paging `convo.load`, search `convo.find` |
| Text as written | `text_delta` (#644) | `MessageDisplay` paragraphs (#623) | `content` every 16 ms | — | Transcript; the live turn while it runs |
| Tool arguments as written | `input_json_delta` (#644) | No | `tool_call` `started` then `running` | Argument deltas (item 4) | Transcript tool row, the argument |
| Thinking | `thinking_tokens` (#644); `set_max_thinking_tokens` in the types, not probed | Transcript | `thinking` blocks | — | Transcript |
| Tool output live | `get_task_output` polled, or the task file (#647) | The task file, deleted on end (#649) | `tool_output_chunk` | — | Live tail under a shell row (Phase 2, `TaskOutput`) |
| Subagent conversation, live | `parent_tool_use_id` lines, prose only under `--forward-subagent-text` (#650) | `subagents/agent-<id>.jsonl`, `SubagentStart`/`Stop` (#656) | REST transcript of the child; progress lines on the parent | A child that can be watched live (item 2) | Tasks sheet: select a subagent, its conversation live (Phase 2) |
| Tasks, with parents | `task_started`, `task_updated`, `background_tasks_changed` | Transcript fold, `Stop.background_tasks`, `PreToolUse` (#656) | `snapshot.subagents`, tasks REST | Shell tasks in the snapshot (item 3) | Activity bar and tasks sheet (Phase 2) |
| Diffs, images, files | Tool inputs and results | Transcript | `diff`, `image` | — | Transcript |
| Hook activity | `hook_started`, `hook_response` (#644) | No | n/a | — | Transcript (Phase 5) |
| Compaction, notices | `compact_boundary` as a transcript record (#259, #312); `informational`, `notification` in the types, not probed | Transcript, `PreCompact`/`PostCompact` | `notice` | — | Transcript; compaction shows in the facts line |

## Dialogs

| TUI feature | Claude, no terminal | Claude, in a terminal | muffit today | muffit, to add (Phase 3) | Where the pane draws it / why not |
|---|---|---|---|---|---|
| Permission: allow, always, edit input | `can_use_tool` with `permission_suggestions` (#645) | The dialog; `PermissionRequest` hook fires for every one (#623); "always" is `updatedPermissions` set to the request's own suggestions (#625); a dialog answered at the keyboard leaves its hook running (#626) | `PermissionPrompt`: Allow / Always allow / Deny | Suggestions for "always"; edited input | Ask dock (Phase 2): the full command, allow / deny / always, then deny-with-reason and edit input (`EditInput` draws the dock's second row) |
| Question, several answers, free text | `can_use_tool` on `AskUserQuestion`, answered through `updatedInput.answers` (#645) | The dialog; bare digit answers it, in an unfocused pane too (#413); free text reaches the model as that text (#639) | No: its own gap 1 | A structured ask tool (item 1) | Ask dock (Phase 2), with the options and a free-text answer |
| Plan approval | `can_use_tool` on `ExitPlanMode`; probe: blocked | The dialog; `PermissionRequest` with `{plan, planFilePath}` and no suggestions (#635) | No | With the ask tool (item 1) | Ask dock (Phase 2) |
| MCP form, URL | `elicitation`; probe: blocked | Terminal only (#637) | No | — | Terminal only — the form is drawn in the TUI; the pane names it and opens the terminal |
| A subagent's ask, beside the main one | `can_use_tool` carrying the agent; probe: blocked | Hook with `agent_id` (#636); it stays answerable while the main session fires `Stop` (#657) | Local to the session: its gap 4 | Child and mesh prompts (item 2) | Ask dock (Phase 2): says "1 of 2", names the next and whose it is |
| Trust prompt, at startup | n/a — a headless session starts trusted or not at all | Terminal only: drawn before a conversation exists; a key at it waits the 8 s boot hold (#549); 2.1.282 draws no numbers and defaults to No, exit (#550); a bare digit answers the numbered form (#72, #106) | n/a | — | Terminal only — the pane sees it on screen and can open a terminal |

## Steering and keys

| TUI feature | Claude, no terminal | Claude, in a terminal | muffit today | muffit, to add (Phase 3) | Where the pane draws it / why not |
|---|---|---|---|---|---|
| Send, images | `user` line; images: probe: blocked | Keys, paste (#619, #620) | `input` with `images` | — | Composer send (Phase 2); attach for images |
| Send, files | `user` line; file attachments over stdin: not probed (docs/20) | — | — | — | Composer send (Phase 2); attach for files — docs/21 draws the attach control for `Files`, on its own |
| Queue, send now, unqueue | `priority`, `cancel_async_message`; probe: blocked | Keys | `input` steers a running turn | Unqueue (item 4) | Composer: while a turn runs the button queues, a second sends now (Phase 2, `QueueNext`/`SendNow`) |
| Interrupt | `interrupt` (#646) | Esc (#610) | `interrupt` (WS and REST) | — | Composer's Stop, drawn when `Interrupt` is declared |
| Continue, pause, resume, stop | Stop is ending the process | Keys | `continue`, `pause`, `resume`, `stop` | — | Session sheet actions (Phase 2) |
| Background, stop a task | `background_tasks` backgrounds the running one, as ctrl+b does (#648); `stop_task` (#669) | ctrl+b (#648) | No | Both (item 3) | Tasks sheet: background, stop (Phase 2) |
| Compact, clear | `/compact`, `/clear` as input; probe: blocked | Typed | `context.compact` | Clear (item 4) | Session sheet actions (Phase 2) |
| Rewind | `rewind_files` (#669) | Esc Esc; the menu is offered as a question (#563) | `revert`, `rewind`, `changesets` | — | Session sheet actions (Phase 2) |
| Fork, rename | `--fork-session`; `rename_session` (#669) | Typed | No | Both (item 4) | Session sheet actions (Phase 2); a resume of a live id is refused (#655) |
| An aside that stays out of the history | probe: blocked | `/btw` typed | `btw` | — | Session sheet actions (Phase 2) |
| Model: list, switch | `list_models` (#668), `set_model` (#669) | `/model` typed | `models.list`, `model.swap` | Adding a provider's model from a client (item 5) | Controls sheet, a choice row (Phase 2); the model in the facts line |
| Effort, thinking | `set_max_thinking_tokens`, `apply_flag_settings`, both in the types, not probed | Typed | `thinking.set` | — | Controls sheet (Phase 2); effort in the facts line |
| Permission mode, rules | `set_permission_mode` (#646, #669), `list_permission_rules` (#668), `update_settings` in the types, not probed | shift+tab | `mode.set`, `permissions.set` | — | The mode as a tappable word in the composer (Phase 2); bypass is always on offer — every launch passes `--allow-dangerously-skip-permissions` |
| Directories | `register_repo_root` (#669; whether it is what `/add-dir` does: probe: blocked) | `/add-dir` | No | — | Session sheet actions (Phase 2) |
| MCP servers | `mcp_status` (#668), `mcp_reconnect` (#669); `mcp_toggle`, `mcp_set_servers` in the types, not probed | `/mcp` | `mcp.*` | — | Controls sheet (Phase 2) |
| Slash commands, with completion | `initialize.commands` (the list below) | Typed | No list | A command list (item 5) | Session sheet: the source's commands as a list; `/` completion in the composer (Phase 2) |
| `@` file completion | `file_suggestions` (#668) | Keys | No | — | Composer's `@` (Phase 2) |
| Enter submits the box | `user` line | A CR in the same write as the text does not submit (#79); submits from 2.1.268 (#536); a CR milliseconds after a retake is kept as a newline, 300 ms later it submits (#619) | Text plus `"\r"` as one `input` (docs/18) | — | Composer send (Phase 2); for a terminal pane the CR timing is the pane's to get right (#619) |
| Arrows and `\x7f` edit the box | n/a | One character at a time, across wrapped rows (#564) | Keys go to the TUI in its terminal; over the pane, everything but text and ctrl-c/esc is dropped (docs/18) | — | The composer is a native input; the mirror reads the box back (#558, #622, #663) |
| Wheel scrolls the transcript | n/a | Past either end it does nothing (#613) | Keys go to the TUI | — | The transcript scrolls (Phase 2) |
| Cursor keys recall prompt history | n/a | `ESC O A` puts the last prompt in the box (#613) | Keys go to the TUI | — | Composer history — not drawn today |
| A bare digit answers a dialog | n/a | #72, #106; an `AskUserQuestion` in an unfocused pane too (#413) | The answer is REST `permission`, never keystrokes (docs/18) | — | Ask dock buttons (Phase 2); for a terminal pane the digit goes in as a key |
| Bracketed paste: an image path becomes `[Image #n]` | Images in the `user` line; probe: blocked | The token is atomic, one `\x7f` takes it (#620); writes over ~800 characters back to back become a paste placeholder (#666); the record wraps pasted text in `<pasted_content>` (#665) | `images` on `input`; a pasted path is typed as a bracketed paste into the TUI (docs/18) | — | Composer attach (Phase 2, `Images`); a pasted image path is sent as an `images` entry and leaves its label in the text (docs/18) |
| The pane sees Claude's composer | n/a | A half-typed line reaches the conversation on every keystroke (#559); a key at the desk arrives as `convo.composer` in ~48 ms (#566); wrapping is `wrap-ansi` hard at W−4 (#662); a space is not drawn (#663); a blank line is a blank row (#622) | n/a | — | `convo.composer` today, for terminal panes |

## Claude's slash commands

The 63 commands in this login's `initialize` list (Claude 2.1.326, no turn sent, #672).
`initialize` carries a separate `terminal_slash_commands` field (#644) that names four of them;
the four names themselves — `doctor`, `color`, `focus`, `reload-plugins` — have no log row.
Phase 8 writes them down as terminal-only.
Muffit has no command list at all (docs/21: "No list"); where a control verb is the closest
thing it is named. The pane draws every non-terminal command in the session sheet's command
list, sent as input (Phase 2).

| Command | What it is | Where the pane draws it / why not |
|---|---|---|
| `clear` (aliases `reset`, `new`) | Start a new session with empty context | Session sheet action (Phase 2); "Compact, clear" above. muffit: to add, clear (Phase 3 item 4) |
| `compact` | Free up context by summarizing so far | Session sheet action (Phase 2). muffit today: `context.compact` |
| `context` | Show current context usage | Facts line (Phase 2). muffit today: `context` |
| `model` | Set the AI model | Controls sheet (Phase 2). muffit today: `models.list`, `model.swap` |
| `usage` (aliases `cost`, `stats`) | Session cost, plan usage, limits | Session sheet and facts line (Phase 2). muffit today: `cost` |
| `config` (alias `settings`) | Set a setting by key | Controls sheet (Phase 2); a settings listing is Phase 3 item 5 |
| `effort` | Set the model's effort level | Controls sheet (Phase 2); effort in the facts line |
| `fast` | Toggle fast mode | Controls sheet (Phase 2). A money-costing control: fast mode is paid from usage credits, not the plan's included usage (#671), so a refusal under `overage = "refuse"` |
| `autocompact` | Set how full the context gets before auto-summarizing | Controls sheet (Phase 2) |
| `output-style` | List or switch output styles | Controls sheet (Phase 2) |
| `mcp` | Manage MCP servers | Controls sheet (Phase 2). muffit today: `mcp.*` |
| `rename` (alias `name`) | Rename the current conversation | Session sheet action (Phase 2); `rename_session` (#669). muffit: to add (Phase 3 item 4) |
| `init` | Initialize a CLAUDE.md with codebase documentation | Session sheet action (Phase 2), sent as input |
| `import` | Import config from another AI coding agent | Session sheet action (Phase 2), sent as input |
| `import-memory` | Import a memory export from another assistant | Session sheet action (Phase 2), sent as input |
| `reload-skills` | Pick up skills added or changed on disk | Session sheet action (Phase 2), sent as input |
| `doctor` | Health-check the install and fix issues | **Terminal only** — in `terminal_slash_commands` (#644); it acts on the install and the TUI |
| `color` | Set the prompt bar color | **Terminal only** — in `terminal_slash_commands` (#644); the bar is a TUI element |
| `focus` | Toggle focus view | **Terminal only** — in `terminal_slash_commands` (#644); it reshapes the TUI's own screen |
| `reload-plugins` | Activate pending plugin changes | **Terminal only** — in `terminal_slash_commands` (#644); plugins reshape the TUI process |
| `heapdump` | Dump the JS heap to the Desktop | Session sheet action (Phase 2), sent as input; it writes a file on the host |
| `debug` | Enable debug logging for this session | Session sheet action (Phase 2), sent as input |
| `code-review` (alias `review`) | Review the current diff or a PR target | Session sheet action (Phase 2), sent as input |
| `security-review` | Security review of the pending changes | Session sheet action (Phase 2), sent as input |
| `simplify` | Review changed code for reuse and simplification, apply the fixes | Session sheet action (Phase 2), sent as input |
| `verify` | Exercise a change end-to-end and observe behaviour | Session sheet action (Phase 2), sent as input |
| `run` | Launch and drive the project's app | Session sheet action (Phase 2), sent as input |
| `deep-research` | Deep research harness: fan-out searches, verified claims, cited report | Session sheet action (Phase 2), sent as input |
| `batch` | Plan a large change, execute it across 5–30 worktree agents | Session sheet action (Phase 2), sent as input; its agents are tasks (activity bar, Phase 2) |
| `loop` (alias `proactive`) | Run a prompt or command on a recurring interval | Session sheet action (Phase 2), sent as input |
| `schedule` (alias `routines`) | Create, update, list or run scheduled cloud agents | Session sheet action (Phase 2), sent as input |
| `goal` | Set a goal — keep working until it is met | Session sheet action (Phase 2), sent as input |
| `recap` | Generate a one-line session recap | Session sheet action (Phase 2), sent as input |
| `insights` | A report analyzing your sessions | Session sheet action (Phase 2), sent as input |
| `skill-doctor` | Show which loaded skills are unused | Session sheet action (Phase 2), sent as input |
| `fewer-permission-prompts` | Scan transcripts, add a prioritized allowlist | Session sheet action (Phase 2), sent as input |
| `auto-mode-setup` | Teach auto mode about your environment | Session sheet action (Phase 2), sent as input |
| `advisor` | Let Claude consult a stronger model at key moments | Session sheet action (Phase 2), sent as input |
| `list-agents` (alias `peers`) | List subagents, teammates, sessions you can message | Session sheet action (Phase 2), sent as input |
| `team-onboarding` | A ramp guide for teammates from your usage | Session sheet action (Phase 2), sent as input |
| `agents` | Listed as `(removed)` in this probe's list | Nothing to draw |
| `design` | Grant or revoke Claude agent access to your Design projects | Session sheet action (Phase 2); a consent decision, sent as input |
| `design-consent` | Grant Design access | Session sheet action (Phase 2), sent as input |
| `design-revoke` | Revoke Design access | Session sheet action (Phase 2), sent as input |
| `design-sync` | Push a React design system to claude.ai/design | Session sheet action (Phase 2), sent as input |
| `claude-api` | Reference for the Claude API / SDK | Session sheet action (Phase 2), sent as input |
| `workflow-authoring` | Reference for writing Workflow tool scripts | Session sheet action (Phase 2), sent as input |
| `__remote-workflow` | Run the workflow script delivered in a server-launched session | Not offered: server-launched sessions only, and Kampr's sessions are not |
| `workflow-launch-exec` | Execute a server-launched workflow handoff | Not offered: server-launched sessions only |
| `plugin-authoring` | Make a change to Claude's own interface or behaviour | Session sheet action (Phase 2), sent as input |
| `run-skill-generator` | Author a run-<unit> skill | Session sheet action (Phase 2), sent as input |
| `update-config` | Configure the harness via settings.json | Session sheet action (Phase 2), sent as input |
| `dataviz` | Create a chart, graph, plot or dashboard | Session sheet action (Phase 2), sent as input |
| `morning` | Render the morning brief as an HTML artifact | Session sheet action (Phase 2), sent as input |
| `skill-creator` | Create, modify and measure skills | Session sheet action (Phase 2), sent as input |
| `docs` | Editable shared documents (skill) | Session sheet action (Phase 2), sent as input |
| `docx` | Word documents (skill) | Session sheet action (Phase 2), sent as input |
| `pdf` | PDF files (skill) | Session sheet action (Phase 2), sent as input |
| `pptx` | PowerPoint (skill) | Session sheet action (Phase 2), sent as input |
| `xlsx` | Spreadsheets (skill) | Session sheet action (Phase 2), sent as input |
| `google-workspace` | Google Drive, Docs, Sheets, Slides connector (skill) | Session sheet action (Phase 2), sent as input |
| `kampr-review` (project) | House style and review priorities for this repository | Session sheet action (Phase 2), sent as input |
| `probe-herdr` (project) | Look up or establish a fact about herdr's behaviour | Session sheet action (Phase 2), sent as input |

Not in this login's list, named in the plan: `login` (a headless session reports that it is
logged out and cannot fix it — docs/20; expected in the list when a session is logged out —
undetermined), `add-dir`, `resume`, `btw`, `permissions`, `status`, `help`. Which of these are state-dependent and which are absent is undetermined:
probe when the plan runs turns again (#670).

## Status and facts

| TUI feature | Claude, no terminal | Claude, in a terminal | muffit today | muffit, to add (Phase 3) | Where the pane draws it / why not |
|---|---|---|---|---|---|
| Account and billing | `initialize.account`: a plan login reports `subscriptionType`, a key an `apiKeySource`, a logged-out session `tokenSource: "none"` (#661, #668) | A headless sibling asked with no prompt (#658) | Provider and model in the session | Which provider and key a session is billed to (item 6) | Session sheet, billing and limits first (Phase 2) |
| Limits | `rate_limit_event`, `get_usage` (#658, #668) | Sibling (#658) | No | Provider rate-limit state (item 6) | Facts line: the five-hour and weekly limits (Phase 2) |
| Context, cost | `get_context_usage`, `get_session_cost` (#668) | Transcript usage | `context`, `cost` | — | Facts line (Phase 2) |
| Trouble and retries | `api_retry`, `assistant.error`; a refused turn ends `is_error: true`, `terminal_reason: "api_error"` (#651) | Transcript and `StopFailure` (#652) | `error` block, `turn` `failed` | Retry attempts as state | Trouble banner: one line and one button, retrying with attempt and countdown (Phase 2) |
| Run state | `status` (#644); `session_state_changed` in the types, not probed | The session file's `status`, four values plus a reason (#344); `waitingFor` distinguishes a permission prompt from a question, 3–20 ms behind the screen (#610) | `state`, mapped to `agent_status` (docs/18) | — | The header's status mark; blocked opens the ask dock; `done` reads idle after `done.read` |
| Fast-mode billing | #671 (Anthropic's docs): on a subscription plan fast mode is paid from usage credits, not the plan's included usage | Same | n/a | — | A money-costing control: a refusal under `overage = "refuse"` |
| Window title: busy glyph plus the session's title | n/a | Terminal only — OSC 0; the glyph tells busy from not-busy and nothing else (#602); answering `OSC 7501` drops the prefix, drawing is unchanged (#601) | n/a | — | Terminal only — a terminal signal; the pane's equivalent is the header's status mark |
| Desktop notifications | n/a | Terminal only — OSC 9 / 99 / 777 / BEL, about 6 s late and only flushed with a repaint; needs a recognised `TERM_PROGRAM` or a `preferredNotifChannel` (#603) | n/a | — | Terminal only — a terminal signal; the pane's equivalent is triage and push |
| Hand a session to a terminal and back | `claude -r` after stopping the headless process (Phase 5) | `/exit` asks before it stops what is running (#660); three hand-overs left one id, one file, no branch (#659) | A terminal running the TUI is the session's pane (docs/18) | — | Offered at rest; otherwise the pane names what stops, stops it, and starts `claude -r` (Phase 5) |

## Not determined

- The seven commands above that are not in this login's list (`login`, `add-dir`, `resume`,
  `btw`, `permissions`, `status`, `help`): state-dependent or absent.
- Every cell marked "probe: blocked": the plan login refuses turns (#670).
- Muffit's TUI keybindings: there is no listing to read them from until Phase 3 item 5 (the
  control and action listing verb, tested against muffit's own TUI).
- What `register_repo_root` does relative to `/add-dir` (docs/21 Phase 0).
- Whether the command list changes with the model, the project, or a logged-out login.