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

Built and committed so far: s1 (Phase 1), s2 (docs/22), s6 (Phase 2a, be4c2a9d), and **Phase 4 is
CLOSED** (s3 pipe mode b3b5411c + s7 fixes 278e74b5 + s10 re-review fix 36622778; kampr-keep
48/48; two reviews passed). **Phase 2a review (s13) passed (SOUND)** — 2b and 5 are unblocked and
launching. Now in flight:

- **s4 — docs/22 review: DONE** — found 11 defects: citation mismatches (several cells cite #644
  for items not in it; a CR-in-same-write cell cites the row that refutes it), a **missing probe-log
  row** for the 2.1.326 no-turn `initialize` (63-command list) the table rests on, and coverage holes
  (the `Files` input ability, the 4 terminal command names, the pane-pieces list shows 10 of 12).
  Fix in flight (s16).
- **s16 — docs/22 fixes** (from s4's 11 findings): correct the citations / mark not-probed with the
  doc's idiom, append the 2.1.326 initialize row if genuinely absent (zero-cost no-turn probe only),
  add the `Files`-ability row. Docs + one probe-log row; no Rust. Queued for a worker slot.
- **s15 — Phase 5 (kampr-claude, the headless source)**: a new `kampr-claude` crate — the
  stream-json reader/writer, `launch.rs` (scrubbed env + billing guards 1–5), `ClaudeSource`
  (create/resume/end/hand-to-terminal) + `ClaudeConversation` (lines→`Update`s, `Command`s→lines,
  guards 6–9), wired into the node's `conversations` registry. **Rust only** — the client
  "new conversation" opener lands after 2b (s15) so it builds on the new pane. **Money-critical: no
  test reaches Anthropic** — a mock `ANTHROPIC_BASE_URL` (happy SSE + error path) + throwaway
  `CLAUDE_CONFIG_DIR` + `billing = any` + `KAMPR_TEST=1`, and a guard test that the launcher
  refuses to start under `KAMPR_TEST=1` without a mock. In a worktree (isolated from s11's Rust WIP).
- **s14 — Phase 2b (the client learns the vocabulary, Kotlin)**: mirror the 13 `convo.*`
  `ServerMsg` + 5 `ClientMsg` + widened `answer` in `client/shared/.../wire/`, and build the pane
  pieces in `client/conversation/` (controls sheet, actions palette, task panel, trouble banner,
  billing/limits in the header, composer send/queue/send-now) + the `kampr-tui` mirror. Nothing the
  source doesn't declare is drawn (no empty rows, no disabled controls, no gaps). Client-only
  checks (not the full Rust gate, which s11 is churning).
- **s8 — Phase 3 (muffit widened)**: the seven capabilities in the **muffit repo** (ask tool, live
  child + answered prompts, shell tasks, tool-arg deltas/unqueue/clear/fork/rename, the
  control-and-action listing verb tested against muffit's own TUI, provider/billing + rate-limit
  state, prompt events + typed REST bodies). On its own `conv-sources` branch there; leaves the
  unrelated in-flight muffit changes alone.
- **s11 — Phase 6 (terminal Claude gaps, node-side)**: trouble parsing (`system/api_error`,
  `isApiErrorMessage`, `StopFailure.error`), the unused hooks (`PreToolUse`, `SubagentStart`,
  `SubagentStop`, `Stop.background_tasks`), fork-aware transcript reading, and live shell/subagent
  output data — in `kampr-journal` + `kampr-node`, surfaced over the existing emit path (no
  `wire.rs`, no `client/`).

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
  but found a **new** defect the fix introduced: a spurious `Truncated` when the standing seq
  survives the roll + 2 hygiene items — fix committed (36622778). Re-review (s12) confirmed all
  three fixed-correctly (each test fails without the fix, measured) and no new defect. **CLOSED**
  (kampr-keep 48/48).

Critical path: **Phase 2a** (wire, Rust — committed be4c2a9d, review s13 SOUND) → **Phase 2b**
(client UI, Kotlin — launching, s14) + **Phase 5** (kampr-claude — launching, s15) → **Phase 7**
(pane pump). Phase 4 is **closed**.

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

## Phase 2 — the wire and the client learn the new vocabulary — 2a COMMITTED (be4c2a9d, +1862/−74; 2089 passed); review in flight (s13); 2b (Kotlin) + 5 after

## Phase 3 — muffit widened (muffit repo) — IN FLIGHT (s8)

## Phase 4 — a keeper for a process that speaks lines — CLOSED (b3b5411c + 278e74b5 + 36622778; kampr-keep 48/48; two reviews passed). Note for Phase 5's pipe consumer: a `Truncated { oldest_seq }` does NOT deliver the `oldest_seq` entry itself — treat it as a hard gap boundary.

## Phase 5 — Claude with no terminal — not started

## Phase 6 — Claude in a terminal, the gaps that need no trait — not started

## Phase 7 — the pane pump behind the trait — not started

## Phase 8 — nothing left that needs the terminal — not started

## Phase 9 — a third source — not started

## Screenshots + gates — not started