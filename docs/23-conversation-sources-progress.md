# Conversation sources — progress

Living tracker for the approved plan in [docs/21-conversation-sources.md](./21-conversation-sources.md).
Kept current as phases land. Branch: `conv-sources`.

## Money state (read this first)

The operator's plan login **cannot run turns right now** (probe #670). `initialize`/`get_usage`
report `subscription_type:"max"`, `rate_limits_available:true`, no overage, but every real prompt
fails with `terminal_reason:"api_error"` — "Claude Code can't use your Claude subscription, usually
because a payment is past due". No-turn control requests keep working.

Consequence: **the turn-based Phase 0 probes are blocked** until the plan runs turns again. The
no-turn control shapes and the billing-state finding are measured and logged (#668–#670). The
phases that need no live Claude (1, 2, 4, 5, 7) proceed against the mock endpoint.

## Phase 0 — probes

DONE (no turn spent, rows 668–670):
- Zero-cost `get_usage`/`initialize` check: `max` plan, rate limits available, no overage.
- No-turn read control requests (#668): `list_models`, `file_suggestions`, `mcp_status`,
  `list_permission_rules`, `get_context_usage`, `get_session_cost`.
- No-turn state-change control requests (#669): `set_permission_mode` to/from `bypassPermissions`,
  `rename_session`, `stop_task`, `register_repo_root`, `rewind_files`, `mcp_reconnect`.

BLOCKED on the plan (turn-based headless):
plan approval; images/files on stdin; `priority`/`cancel_async_message`; slash commands as input;
valid `set_model` value shape; `stop_task`/`rewind_files`/`rename_session`/`file_suggestions`/
`mcp_*`/`elicitation` during a turn; `register_repo_root` vs `/add-dir`; a subagent's `can_use_tool`;
`result.origin` on each self-started turn; the `--allow-dangerously-skip-permissions` launch flag.

BLOCKED (keeper lifecycle, need a live Claude under the keeper):
keeper killed mid-turn; node gone while a `can_use_tool` stands; stdin held open with nobody reading stdout.

BLOCKED (terminal + macOS):
a made-to-fail session; two foreground shells and their task files; a forked transcript through
`ClaudeParser`; the exit dialog's "Move to background and exit".

STILL DOABLE WITHOUT A TURN:
- Fast-mode billing from Anthropic's documentation (doc lookup).
- `docs/22-parity.md` (the TUI surface list).
- Phases 1, 2, 4, 5, 7 (pure code, mock endpoint).

## Phase 1 — the seam, and muffit on it — not started

## Phase 2 — the wire and the client learn the new vocabulary — not started

## Phase 3 — muffit widened (muffit repo) — not started

## Phase 4 — a keeper for a process that speaks lines — not started

## Phase 5 — Claude with no terminal — not started

## Phase 6 — Claude in a terminal, the gaps that need no trait — not started

## Phase 7 — the pane pump behind the trait — not started

## Phase 8 — nothing left that needs the terminal — not started

## Phase 9 — a third source — not started

## Screenshots + gates — not started