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
- Fast-mode billing (#671, from Anthropic's docs): on a subscription plan fast mode is paid from
  usage credits, not the plan's included usage — a money-costing control, a refusal under
  `overage = "refuse"`.

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
- `docs/22-parity.md` (the TUI surface list).
- Phases 1, 2, 4, 5, 7 (pure code, mock endpoint).

## Orchestration (in flight)

Built and committed so far: s1 (Phase 1), s2 (docs/22), s3 (Phase 4), s7 (Phase 4 fixes,
278e74b5). Now in flight:

- **s4** — review of docs/22-parity.md.
- **s6 — Phase 2a (the wire, Rust only)**: additive `convo.*` frames in `wire.rs` (both
  directions), the node's `conversations.rs` emitting them from the seam `Update`s and routing
  the new client verbs, and `MuffitConversation` lifting muffit's existing capabilities into the
  new seam `Update`s (controls, actions, tasks, task-output, trouble, context, cost, limits,
  session). No Kotlin (that's Phase 2b).
- **s8 — Phase 3 (muffit widened)**: the seven capabilities in the **muffit repo** (ask tool, live
  child + answered prompts, shell tasks, tool-arg deltas/unqueue/clear/fork/rename, the
  control-and-action listing verb tested against muffit's own TUI, provider/billing + rate-limit
  state, prompt events + typed REST bodies). On its own `conv-sources` branch there; leaves the
  unrelated in-flight muffit changes alone.
- **s12 — Phase 4 re-review (of s10's fix)**: verify the spurious-`Truncated` fix, the
  client-write-cap guard test, and the boundary alignment are genuinely fixed (tests fail without
  the fix) and introduce no new defect. The last gate before Phase 4 closes.

- **s1 — Phase 1 (the seam)**: `kampr-core/src/conversation/` module with `Conversation`/`Source`
  traits + the types; turn/block/facet moved out of `kampr-journal` (parsers stay); `MuffitSource`
  over the existing `kampr-muffit::Follow`; the `conversations` registry in `kampr-node`;
  `session.rs` routing through the registry. No wire changes (that's Phase 2). **DONE, committed**
  (`make test-fast` 2079 passed; the 22 muffit integration tests unmodified; `session.rs` free of
  `kampr_muffit` paths).
- **s2 — Phase 0 `docs/22-parity.md`**: the line-by-line TUI surface list, built from the measured
  probe rows. **DONE, committed** (111 rows; 63 slash commands from a no-turn `initialize`; every
  terminal-only row has a measured reason; the command list is state-dependent — this login omits
  login/add-dir/resume/btw/permissions/status/help, so the plan's 68 is not confirmed).
- **s3 — Phase 4 (the keeper)**: pipe mode in `kampr-keep` — stdin/stdout pipes, `PipeLog` with
  seq numbers, `SeqReader` read-from-seq + live follow, `write` ctl request, mode-aware keeper +
  systemd 2-fd lifecycle. **MERGED** into `conv-sources` (b3b5411c, +507/−57, kampr-keep only);
  `make test-fast` 2072 passed; TDD restart-mid-turn test against a hermetic shell-script fake.
  Review (s5) found 3 defects on the durability path (seq-resume-to-0, `Truncated` never emitted,
  overlong `write` falsely acked) — fixes committed (278e74b5). Re-review (s9) confirmed all three
  fixed-correctly but found a **new** defect the fix introduced: a spurious `Truncated` when the
  standing seq survives the roll (the exact recovery path Phase 4 exists for) + 2 hygiene items —
  fix committed (36622778, kampr-keep 48/48); re-review in flight (s12), the last gate before close.

Critical path: **Phase 2a** (wire, Rust — in flight, s6) → **Phase 2b** (client UI, Kotlin) →
**Phase 5** (kampr-claude) → **Phase 7** (pane pump). Phase 4 (s3 + fixes s7, re-review fix s10) feeds Phase 5.

Phase 2 split on the toolchain boundary for review: **2a (Rust, s6)** adds `convo.abilities`,
`convo.controls`, `convo.actions`, `convo.tasks`, `convo.task`, `convo.trouble`, `convo.limits`,
`convo.context`, `convo.session`, `convo.state`, `convo.live`, `convo.suggested`, `convo.acted` to
`ServerMsg` and `convo.steer`/`convo.set`/`convo.act`/`convo.follow`/`convo.suggest` (+ widened
`answer`) to `ClientMsg` in `crates/kampr-core/src/wire.rs` (additive, serde renamed), maps them
in the node's `conversations.rs`, and lifts muffit's existing capabilities into the seam
`Update`s. **2b (Kotlin, after 2a)** mirrors them in `client/shared/.../wire/` and builds the pane
pieces in `client/conversation/` (controls sheet, actions palette, task panel, trouble banner,
billing/limits in the header) — nothing the source doesn't declare is drawn.

## Phase 1 — the seam, and muffit on it — DONE (s1)

Built as the plan's "The seam" section, verbatim for the trait shapes:

- `kampr-core/src/conversation/`: `mod.rs` (the `Conversation` and `Source` traits, `Emit`,
  `Refusal`, `NewSession`, `Stored`, `Ending`), `command.rs` (`Command`, `When`, `Reply`,
  `SuggestKind`), `update.rs` (`Update`, `Live`, `Notice`, `ConvoFound`), `abilities.rs`,
  `ask.rs`, `control.rs`, `facts.rs`, plus `composer.rs` — the muffit line editor, moved here so
  the node's `Session` can hold one without a muffit path (deviation: an 8th file; see below).
- Turn/block/facet/model types moved to `kampr-core::conversation`; `kampr-journal` re-exports
  every one, parsers unchanged, core no longer depends on journal in the seam's types
  (journal ← core only, through the re-exports).
- `kampr-muffit/src/source.rs`: `MuffitSource` (abilities declared from the capabilities the
  client asks for in its attach hello) and `MuffitConversation`, which wraps `Follow`'s run loop
  unchanged in shape, translating seam `Command`s to `follow::Command`s and `follow::Update`s to
  seam `Update`s. 4 new unit tests on the mapping.
- `kampr-node/src/conversations.rs`: the `Conversations` registry, keyed by pane, one per socket:
  `open` (source → conversation → spawned task), `command` (verbs, non-blocking), `send`
  (composer messages, queued), `take`, `drain`; the `Update` → wire-message mapping lives here.
- `kampr-node/src/sessions.rs`: `conversation()` (the pane's source + its id in that source's
  namespace + whether a terminal sits beside it) and `answer()` (the REST prompt answer).
- `kampr-node/src/session.rs`: no `kampr_muffit` path left — `watch` routes every conversation
  pane through `Conversations::open`, `convo.sub`/`convo.find`/`convo.load` through
  `Conversations::command`, input through `crate::muffit::input` (composer line → seam command,
  REST fallback when no conversation is open), answers through `SessionNode::answer`.

Deviations, all forced by the "no wire change" + "tests unmodified" constraints:

- `composer.rs` is an 8th file in `kampr-core/src/conversation/` (the plan lists seven). The
  line editor is a seam-shape type the node's `Session.lines` must hold; keeping it in
  `kampr-muffit` would put a muffit path back in `session.rs`. `kampr_muffit::input` survives as
  a re-export.
- `Source::open` receives the pane **in the source's own namespace** (`m:<id>`), not the node's
  local pane id: a TUI's pane is `n:<id>` in the herd and the daemon knows only `<id>`.
  `SessionNode::conversation()` does the mapping; the registry just forwards.
- `MuffitSource::open` never refuses for a well-named pane — liveness failures surface as
  `Update::Missing`/`Failed` from the run loop, exactly as `Follow` does today (today's
  `watch_muffit` had no open step at all; refusing up front would change what a re-watch of a
  just-stopped session sends).
- The seam's `Update::Asks` maps to the same `Pending` frame as today's `Update::Pending`:
  `id` stays `None`, the three `1/2/3` options, `source: Transcript`. The frame is byte-identical
  to the old one; the seam's wider `Ask` (by, kind, multi) has no wire frame until Phase 2.
- A composer submit for a pane whose conversation task has ended (link still registered) now
  falls through to the REST path instead of erroring "the conversation closed" — the daemon
  still answers, and the tests do not reach the race.

## Phase 2 — the wire and the client learn the new vocabulary — 2a IN FLIGHT (s6, Rust); 2b (Kotlin) after

## Phase 3 — muffit widened (muffit repo) — IN FLIGHT (s8)

## Phase 4 — a keeper for a process that speaks lines — b3b5411c + fixes 278e74b5 + re-review fix 36622778 (kampr-keep 48/48); re-review in flight (s12), last gate before close

## Phase 5 — Claude with no terminal — not started

## Phase 6 — Claude in a terminal, the gaps that need no trait — not started

## Phase 7 — the pane pump behind the trait — not started

## Phase 8 — nothing left that needs the terminal — not started

## Phase 9 — a third source — not started

## Screenshots + gates — not started