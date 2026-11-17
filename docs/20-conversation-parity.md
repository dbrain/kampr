# A conversation pane that needs no terminal

What Claude Code will say about a session without anyone reading its screen, measured on 2.1.311,
and what the conversation pane would have to show before the terminal beside it is optional.

This is a findings document, not a plan. Nothing here is built. Every claim about Claude is a row
in the [probe log](./03-probe-log.md), #644 to #661, or an earlier row cited where it is used; the
scripts are in `research/probe/claude-stream-json/`.

## Two ways to hear a Claude

**Headless, over stream-json.** `claude -p --input-format stream-json --output-format stream-json`
is the unmodified binary with no TUI. It writes one JSON object a line on stdout and reads the same
on stdin. This is what the Agent SDK speaks; the SDK is a wrapper that spawns the binary and frames
these lines, and it authenticates however the binary does, so on a claude.ai login it is
subscription usage (`apiKeySource: "none"`, #644). Kampr would have to start such a session itself:
there is no attaching to one somebody started in a terminal.

**Beside a TUI.** An interactive Claude has no single feed. What it leaves is the transcript on
disk, the hooks Kampr already installs ([docs/12 W11](./12-harness-briefs.md)), a status file per
process, and, new in these probes, a live output file per running shell (#649).

| | Headless stream-json | Beside a TUI |
|---|---|---|
| Finished messages, tool calls, tool results | `assistant` and `user` lines (#644) | The transcript, the same records |
| Text as it is written | `text_delta` per token (#644) | `MessageDisplay` hook, a paragraph at a time (#623 onward) |
| Tool arguments as they are written | `input_json_delta` (#644) | Nothing |
| Permission, question, plan | `can_use_tool` request, answered on stdin (#645) | `PermissionRequest` hook (#623, #624, #635 onward) |
| Interrupt, change mode, change model | Control requests (#646) | Keystrokes |
| Sending a prompt | A `user` line on stdin | Keystrokes. The `cc-socks` socket is not a substitute (#653) |
| A shell's output while it runs | `get_task_output`, or its file (#647) | Its file (#649) |
| A subagent's calls and results | Lines tagged `parent_tool_use_id` (#650) | `<session>/subagents/agent-<id>.jsonl` ([docs/12 W2](./12-harness-briefs.md)) |
| A subagent's prose | Only with `--forward-subagent-text` (#650) | The same file |
| An API failure, and each retry | `api_retry`, then `assistant.error` and `result` (#651) | Transcript records and the `StopFailure` hook (#652) |
| Plan limits | `rate_limit_event`, `get_usage` (#644, #646) | Nothing of its own; a headless sibling can be asked (#658) |
| Account, models, slash commands | `initialize` response (#646) | Nothing found |
| Context window use | `get_context_usage` (#646) | Nothing found |
| Cost and timing of a turn | `result` (#644) | Per-record usage in the transcript |

The headless column is complete enough that a session Kampr starts would need no terminal. The TUI
column is complete enough for reading and answering, and has no source at all for limits, account
state or context use.

## Is stream-json something to build on

What Anthropic's documentation says, read on 2026-11-13:

- **The output side is a documented interface.** The [headless page](https://code.claude.com/docs/en/headless.md)
  lists `stream-json` as *"newline-delimited JSON for real-time streaming"*, documents `system/init`
  and `system/api_retry` by shape, and says to feature-detect from the `init` line's `capabilities`
  array *"instead of comparing version strings"*.
- **The input side and the control protocol are documented only through the SDK.** The CLI reference
  lists `--input-format stream-json` and `--permission-prompt-tool`. There is no page for
  `control_request` and `control_response`; the [TypeScript reference](https://code.claude.com/docs/en/agent-sdk/typescript.md)
  mentions the wire shape in passing and says *"if you parse the wire protocol yourself, treat a
  missing field as an older CLI rather than as proof that nothing is pending."* The message schema is
  the SDK's `SDKMessage` type and nothing else.
- **No stability or versioning promise was found for the wire**, and no deprecation of `-p` either.
  The direction is toward it: `claude -p` is described as *"using the Agent SDK via the CLI"*, the
  SDK overview tells other languages to *"run the CLI as a subprocess"*, and streaming input is
  called *"the preferred way to use the Claude Agent SDK."* One announced change: `--bare` *"will
  become the default for `-p` in a future release"*, which skips hooks and CLAUDE.md discovery
  unless asked for.
- **Billing.** The [help article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
  says, as of its 7 October 2026 update, *"when you're signed in with your Claude plan, the Claude
  Agent SDK and claude -p still draw from your subscription limits."* A change to that was announced
  earlier in 2026 and then paused, so it is the part most likely to move.
- **Terms.** *"Anthropic does not permit third-party developers to offer Claude.ai login into their
  own applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on behalf
  of their users"*, and also *"Nor does it prevent an end user from signing in to the unmodified
  Claude Code binary with their own Claude subscription."* ([legal and compliance](https://code.claude.com/docs/en/legal-and-compliance.md)).
  Kampr running the operator's own binary under the operator's own login is the second sentence.

So: supported and moving forward as a product surface, with the SDK's types as the only contract.
A Rust client of the wire would carry the cost of tracking those types by hand. The SDK's type file
for 0.3.323 names 40 message types and 41 control requests; the probes exercised about a third.

## What "feature complete" means

Each line is something a person does at Claude's terminal today. The last column is what was
checked in this repository, not a full audit.

### Reading

| The pane shows | Headless | Beside a TUI | Kampr today |
|---|---|---|---|
| The reply as it is written | `text_delta` | `MessageDisplay` paragraphs | Built on the hook (W11) |
| That Claude is thinking | `system/thinking_tokens` estimates; the block's text is empty (#644) | Not probed | |
| Each tool call and its result | `assistant`, `user` | Transcript | Built |
| **A shell's output while it runs** | `get_task_output` after `task_started`, or the file from the first line (#647) | `<tmp>/claude-<uid>/<cwd>/<session>/tasks/<id>.output` (#649) | The running strip names the shell and shows none of its output |
| **A subagent's conversation** | `parent_tool_use_id` lines, prose with `--forward-subagent-text` (#650) | `subagents/agent-<id>.jsonl`, written continuously (W2) | W2 reads the file; the strip is a list, with no way from it into the agent's output |
| A subagent's own shell | A task with `parent_task_id` (#650) | Not probed | |
| What is running now | `background_tasks_changed`, the whole list each time (#648) | Transcript launches and `<task-notification>` (#418) | Built (W2b) |
| Hook activity | `hook_started`, `hook_response` with `--include-hook-events` (#644) | Nothing | |

### Answering and steering

| The pane can | Headless | Beside a TUI | Kampr today |
|---|---|---|---|
| Allow, deny, always-allow | `can_use_tool` with `permission_suggestions` (#645) | `PermissionRequest` hook (#624, #625) | Built (W11) |
| Answer a question, with free text | `updatedInput.answers` (#645) | The same through the hook (#624, #634) | Built (W11) |
| Approve or revise a plan | Not probed headless | Hook, `allow` with `updatedInput` (#635 onward) | Built (W11) |
| Interrupt a turn | `interrupt` (#646) | Esc | Keystroke |
| Change permission mode | `set_permission_mode` (#646) | shift+tab | Keystroke |
| Background a running command | `background_tasks` (#648) | ctrl+b | Keystroke |
| Stop a background task | `stop_task` in the SDK's types, not probed | Not probed | |
| Run a slash command | `initialize` lists 68 with descriptions (#646); sending one not probed | Typed | Typed |
| Change model or effort | `set_model` in the types, 14 models listed (#646); not probed | `/model` | Typed |
| Answer an MCP server's form | `elicitation` in the types, not probed | Passed through to the terminal (W11) | Terminal only |

### Knowing something is wrong

This is the group with nothing built. `grep` finds no reader of `isApiErrorMessage`, `api_error`
or a rate-limit field anywhere in `crates/` or `client/`, and `kampr-node/src/hooks.rs` handles
`StopFailure` exactly as `Stop`, dropping its `error`.

| The pane says | Headless | Beside a TUI |
|---|---|---|
| Not logged in | `assistant.error: "authentication_failed"`, text *"Not logged in · Please run /login"*, no retry (#651) | Transcript record with `isApiErrorMessage`; `StopFailure {error}` (#652) |
| Login expired or revoked | Two `api_retry` with `error_status: 401`, then `authentication_failed` (#651) | The same pair of sources (#652) |
| Anthropic is overloaded or failing | `api_retry` with `error: "overloaded"` or `"server_error"`, attempt and delay, then the final record with `api_error_status` (#651) | `system/api_error` record per retry with `retryAttempt`, `maxRetries`, `retryInMs` (#652) |
| Rate limited | `error: "rate_limit"` (#651) | The same |
| Retrying, attempt 3 of 10, next in 8 s | `api_retry` | `system/api_error` |
| How close the plan's limits are | `rate_limit_event` on every turn, `get_usage` on demand (#644, #646) | Nothing found |
| Billing, account on hold, organisation not allowed, model not found, output cut at the token limit | Kinds the SDK names; not provoked | Not provoked |

A caution for whoever builds this: the failing turn's `result` line says `subtype: "success"`. The
fields that mean failure are `is_error: true` and `terminal_reason: "api_error"` (#651).

## What still needs a terminal

- **`/login`.** A headless session reports that it is logged out and cannot fix it. Kampr can say
  so and open a terminal; the SDK's `auth_status` message suggests a flow exists, and it was not
  probed.
- **Terminal-only slash commands.** `init` lists them separately as `terminal_slash_commands` (#644).
- **A Claude somebody started by hand** stays on the hooks path and keeps its gaps: no limits, no
  account state, prompts sent as keystrokes. It can be stopped and its session resumed headless,
  but not joined while it runs (#655).
- **Tying a foreground shell's file to its tool call beside a TUI.** The file is named by a task id
  that appears in no transcript record while the command runs (#649). With one foreground command
  at a time the newest file is the answer; with parallel calls it is a guess. Headless has no such
  gap, since `task_started` carries both ids (#647).

## A feed other agents can fill

The headless stream reduces to a small vocabulary, and the other agents Kampr reaches have
something for most of it: muffit's own API ([docs/18](./18-muffit.md)), Codex's `app-server`, and
the Agent Client Protocol for the rest, which is how [Orca](https://github.com/stablyai/orca)
covers them.

| Event | Claude headless | Claude beside a TUI |
|---|---|---|
| Session facts: model, mode, account, commands | `init`, `initialize` | Transcript, `sessions/<pid>.json` |
| Turn state: idle, running, needs an answer | `status`, `session_state_changed` in the types | `sessions/<pid>.json` (#344), hooks |
| Text delta | `text_delta` | `MessageDisplay` |
| Tool call, tool result, each with an optional parent | `assistant`, `user`, `parent_tool_use_id` | Transcript, subagent files |
| Task started, output, ended | `task_started`, `get_task_output`, `task_notification` | Transcript launches, output files |
| Request: permission, question, plan, form | `can_use_tool`, `elicitation` | `PermissionRequest`, `Elicitation` |
| Failure and retry | `api_retry`, `assistant.error` | `system/api_error`, `StopFailure` |
| Limits and usage | `rate_limit_event`, `get_usage`, `result` | None |

Going the other way: prompt, answer to a request, interrupt, set mode, set model, background or
stop a task.

## One session, two processes

A session started headless can be opened in a terminal and the reverse; the id and the transcript
are the same thing to both (#655). `claude -r <id>` on a session that only ever ran over
stream-json draws its turns as ordinary history.

What does not exist is two processes sharing one live conversation. Each loads the transcript once
and then keeps its own copy in memory. A prompt given to the second is never seen by the first; both
append to the one file under the one session id, and the file becomes a tree with two branches from
the same parent. The next resume continued one branch and silently left the other, and in the
measured case it was not the branch written last. Nothing warns: the only notice on screen was
about Remote Control.

So a headless session and a TUI can take turns on a session, and cannot both be open on it.
Taking turns is clean: three hand-overs left one session id, one file and no branch (#659). For
Kampr this is a rule and a hazard:

- Handing a session from the conversation pane to a terminal means stopping the headless process
  first, then `claude -r`. The reverse the same.
- A transcript is not guaranteed to be one line of conversation. Anything that reads it in file
  order will show both branches interleaved once this has happened.

Only `claude --bg` sessions are guarded: resuming a running one is refused with a pointer to
`attach`, `stop` or `--fork-session`.

## Claude's own background sessions

`claude --bg` is a keeper of the kind [docs/17](./17-native-sessions.md) built: a per-user daemon
holds a pty, runs the ordinary interactive Claude on it, and `claude attach` and `claude logs` are
that pty (#654). The session is a TUI in every respect, with hooks, a transcript and a `cc-socks`
inbox, so it is not a third feed. It adds two things worth reading:

- `~/.claude/jobs/<id>/state.json`: `state` (`working`, `done`), a one-line `detail` of what it is
  doing in Claude's own words, `tempo`, the count of tasks in flight, and the launch `intent`.
- `claude agents --json`: every live Claude on the machine, interactive ones included, as
  `{pid, cwd, kind, sessionId, name, status}`. The same facts as `sessions/<pid>.json`, in one call.

## What the hooks leave unused

Kampr installs eleven events (`kampr-node/src/hooks/install.rs`). Measured on a turn with a shell,
a background shell and a subagent (#656), the rest carry these, all of them for a TUI session where
the alternative is waiting for the transcript:

| Event | Carries | What it would give the pane |
|---|---|---|
| `PreToolUse` | `tool_name`, `tool_input`, `tool_use_id`, and `agent_id` when a subagent made the call | The tool card the moment the call starts instead of when its record lands; the start time of a foreground shell, which is the nearest thing to a key for its output file (#649) |
| `SubagentStart` | `agent_id`, `agent_type`, 40 ms after the `Agent` call | The subagent's transcript file name at launch. A synchronous `Agent` otherwise names no agent until it returns ([docs/12 W2](./12-harness-briefs.md)) |
| `SubagentStop` | `agent_transcript_path`, `last_assistant_message`, `background_tasks` | The ending and the path, with no directory scan |
| `Stop` (installed, fields unread) | `background_tasks`: each running shell's `id`, `status`, `description` | Claude's own list of what is still running when the turn ends, against the one `running.rs` folds from the transcript |
| `StopFailure` (installed, `error` unread) | The error kind and the sentence (#652) | The whole "something is wrong" group above |
| `PostToolUse` (installed, used only to release a card) | `tool_response`, `duration_ms` | The result before the transcript has it |
| `CwdChanged` | `old_cwd`, `new_cwd` | The pane's directory without procfs |
| `SessionStart` | `source` | Startup against resume against clear |
| `Notification`, `PreCompact`, `PostCompact`, `PostModelSwitch` | Not fired in the probe | Idle and attention notices, compaction, model changes |

A background task finishing arrives as a `UserPromptSubmit` whose `prompt` is the
`<task-notification>` block (#656). As `convo.heard` that is harmless, since a client only matches
it against text it sent. But it, and the main agent's `Stop`, fire while a background subagent's
request is still held (#657), and `hooks.rs` used to let every held request go on either. It now
lets go only the main agent's.

A headless Claude also answers `get_usage` with no prompt sent (#658), which gives a terminal
session's limits and account from the side. [docs/21](./21-conversation-sources.md) is the path
forward from all of this.

## Not measured

- An interactive session made to fail: #652's transcript records and hook payloads came from
  headless runs.
- Whether a headless session started outside another Claude differs. Every headless probe inherited
  `CLAUDE_CODE_CHILD_SESSION` from the session that ran it; an interactive child with that marker
  binds no socket and writes no session file (#653).
- Plan approval, `set_model`, `stop_task`, elicitation, slash commands and file attachments over
  stdin.
- What picks the branch a resume continues when a transcript has two (#655).
- The protocol of a background session's pty socket, and `claude attach` (#654).
- Whether an HTTP hook, which the hooks guide lists as a type, would spare the process a command
  hook costs: 208 ms a time on the Mac (#643).
- Whether `tool_progress` ever carries anything for a shell. None was emitted in these runs.
- macOS, where the temporary directory and the socket path differ.
