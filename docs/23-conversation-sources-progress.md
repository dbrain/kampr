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
  row** for the 2.1.326 no-turn `initialize` (63-command list), and coverage holes (the `Files` input
  ability, the 4 terminal command names, the pane-pieces list 10-of-12). **Fixed by s16.**
- **s16 — docs/22 fixes: DONE** (c365d408, docs only) — all 11 fixed; the 63-command list was
  genuinely absent so it re-ran the zero-cost no-turn `initialize` and **appended probe row #672**
  (63/63 names verified); `kampr-testkit` probe-log guard green (1..672, no gaps, all `#N` resolve).
  **docs/22 review loop CLOSED.**
- **s17 — Phase 6 review: DEFECTS FOUND** — trouble (6-kind classify, `from_result` on
  `is_error`+`terminal_reason`), 14 hooks, fork-aware read, and no-wire-edits all **CONFIRMED**; red
  tests re-verified (8/11 trouble, 2/3 fork, both hooks fail without the fix); the `live::reopening`
  flake is **pre-existing** (fails in the full suite on base `207ebe80` too, passes in isolation).
  **One blocking defect:** `tasks_tick` scans the wrong dir for live shell output
  (`~/.claude/projects/<slug>/<session>/tasks/` vs the real `/tmp/claude-<uid>/<cwd-slug>/<session>/tasks/`
  per #649/#310) — and its test writes to the code's own path so it **passes with the defect**
  (rule-#2 violation). Fix in flight (s18): real dir + `bind()` `facts.bg` exclusion + test rewritten
  to the real path + probe row.
- **s15 — Phase 5 (kampr-claude, the headless source): DONE** — cherry-picked `ee203d76` (15 files,
  +3031) onto conv-sources (the worktree branch's `dc8f4815` base was s11's Phase 6 WIP, already
  committed, so only the Phase 5 commit was taken). The `kampr-claude` crate: `wire.rs` (lenient
  stream-json reader + control-request writer, request ids matched), `launch.rs` (scrubbed env +
  guards 1–5 + guard 11), `billing.rs` (guards 3, 6–9), `conversation.rs` (lines→`Update`s,
  `Command`s→lines), `source.rs`/`provider.rs` (`ClaudeSource` + `Provider` + `Herd`, pipe mode).
  `mock.py` (self-contained ThreadingHTTPServer; **logs every request** so a test reaching the mock
  is provably not reaching Anthropic). **Every guard has a test that fails without it** (unit + e2e
  real `claude -p` vs the mock); `cargo test -p kampr-claude` 34 passed, zero warnings. **Deviation
  (justified):** guard-2 scrub *blanks* provider-shaped names (the keeper pipe mode can't do a pure
  allow-list — Phase 4 closed); `--forward-subagent-text` added. Review in flight (s19, money-safety).
- **s19 — Phase 5 review (money-safety)**: in a worktree, isolated build dir — verify no test
  reaches Anthropic (guard 11), the guard-2 scrub-blanking is sound (stray key doesn't cross under
  `plan`, mock preserved, keys cross under `api`), guards 3/4/6/7/8/9 red-verified, guard 5 never
  `--bare`, lenient reader, node wiring, and no wire/client/journal/keep edits.
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

## Phase 5 — Claude with no terminal — COMMITTED (cherry-pick `ee203d76`, `kampr-claude` crate + node wiring + `mock.py`): stream-json reader/writer, `launch.rs` scrubbed env + guards 1–5, `billing.rs` guards 3/6–9, `ClaudeSource`/`Provider`/`Herd` (pipe mode), every guard red-tested (34 passed, no test reaches Anthropic). Deviation: guard-2 scrub *blanks* provider names (keeper pipe mode). **Review in flight (s19, money-safety).**

## Phase 6 — Claude in a terminal, the gaps that need no trait — COMMITTED (18c61685, 13 files +1638/−34): trouble parsing (6-kind classify, `StopFailure.error`), 14 hooks installed (PreToolUse/SubagentStart/SubagentStop/`Stop.background_tasks`), fork-aware transcript read, live shell output + live subagent conversations — all through **existing** frames (`convo.trouble`/`convo.task`/`convo.tasks`/`convo.sub`), no wire/client edits. Measured red tests (8/11 trouble, both fork, live shell+subagent). **Review s17: DEFECTS FOUND** — 1 blocking (`tasks_tick` scans the wrong shell-task dir; its test passes with the defect); trouble/hooks/fork all confirmed sound. **Fix in flight (s18).** Sibling-limits side-channel (guard 10/#658) deferred to Phase 5.

## Phase 7 — the pane pump behind the trait — not started

## Phase 8 — nothing left that needs the terminal — not started

## Phase 9 — a third source — not started

## Screenshots + gates — not started