# Conversation sources — progress

Living tracker for the approved plan in [docs/21-conversation-sources.md](./21-conversation-sources.md).
Kept current as phases land. Branch: `conv-sources`.

## Money state (read this first)

The operator's plan **runs turns again** (payment fixed; the refusal of #670 is gone). `initialize`
reports `subscriptionType:"Claude Pro"` and `get_usage` shows 5 h 72% / 7 d 47%, no overage, as of
this session. The turn-based Phase 0 probes are done (rows 674–684 below).

## Final report

**Status: ready for user testing.** All ten phases are built, reviewed (each review loop closed),
and committed on `conv-sources`. The gates are green and **stable** (s39 root-caused and fixed the
last pre-existing flaky live-herdr tests). The money-safety item — the billing guards — is verified
against the operator's real plan login to the extent the plan's usage window allows (see below).

### What was built, by phase
- **0 — probes.** Everything the plan lists as "not measured" is measured: no-turn control shapes
  and billing state (668–670), the full turn-based set (674–684: plan approval, images/files,
  priority/cancel, slash commands, `set_model` shape, mid-turn controls, `register_repo_root`,
  subagent `can_use_tool`, `result.origin`, the bypass launch flag, elicitation), and the Codex
  app-server protocol (26-row log). Each is a probe-log row; the log is the only source of facts.
- **1 — the seam.** One `Conversation`/`Source` trait in `kampr-core/src/conversation/` in front of
  every agent, with a capabilities definition; muffit is the first source on it; the node's
  `conversations` registry replaces the muffit special cases. `session.rs` has no muffit path.
- **2 — the wire + client.** 13 `convo.*` server frames and 5 client verbs, all additive (an older
  client/node keeps working). The Kotlin client mirrors them field-for-field and the pane draws
  only what the source declares — nothing with nothing to show takes no room.
- **3 — muffit widened.** All seven capabilities in the muffit repo (ask tool, live child, shell
  tasks, arg-deltas/steer, the command catalog, billed-to + rate-limit, prompt events).
- **4 — the keeper.** Pipe mode beside pty mode: stdin/stdout pipes, a seq-numbered `PipeLog`,
  read-from-seq, and the 2-fd lifecycle. A node restart mid-turn comes back to the same request.
- **5 — Claude with no terminal.** The `kampr-claude` crate: the stream-json reader/writer, the
  scrubbed launch, the 12 billing guards, and `ClaudeSource`/`Provider` so the herd lists headless
  sessions as terminal-less panes. **5b** then closed the headless gaps Phase 8 found: input/answer
  routing to the conversation, interrupt, context usage, completions, queue/send-now/unqueue,
  files, subagent-live, and search/older.
- **6 — Claude in a terminal.** The gaps that need no trait: trouble parsing, the 14 hooks,
  fork-aware transcript read, and live shell/subagent output — all over existing frames.
- **7 — the pane pump behind the trait.** `TerminalClaude: Conversation` in `convo.rs`; the other
  harnesses (Codex, omp, pi, agy) wrapped with a smaller ability set.
- **8 — the parity pass.** Every row of docs/22 checked against the code; it found and drove the
  fix for the CRITICAL headless routing bug (send/answer were drawn but non-functional).
- **9 — a third source.** The Codex app-server (`kampr-codex`): NDJSON JSON-RPC over stdio,
  pipelined for the one-behind flush, mapped to the same seam.

### Billing guards (the part that can hurt)
- **Guards 3, 6, 9 — verified against the real login, no turn spent** (probe row 685): `initialize`
  returns the plan account with no `apiKeySource`; `get_usage` shows no overage; billing reports as
  `Plan`.
- **Guards 2, 4 — the turn-side half is staged, not spent.** Both need one haiku turn (a
  parent-env key blanked by the scrub; the turn's `system/init` `apiKeySource: "none"`). The 5-hour
  window was at 94%, above the ~90% line, so the money discipline held the turn. Run
  `cargo run -p kampr-claude --example plan-probe -- turn --with-key` after the window resets
  (2026-11-27T07:09:59Z) to close it out.
- **All negative cases are mock-tested and passing** (a key in the env, a logged-out config, the
  bare default, an `isUsingOverage` mid-turn, the unattended cap, the launcher refusing under
  `KAMPR_TEST=1` with no mock). No test reaches Anthropic.

### Skipped or deferred, and why
- **Codex real-world** — needs a Codex subscription; the protocol is pinned by the probe log +
  mock, so the source is verified offline.
- **Keeper lifecycle probes** (mid-turn kill, orphaned `can_use_tool`, stdin held) — need a
  herdr/keeper environment.
- **macOS terminal probes** — need macOS.
- **`kampr-tui` mirror** (the Rust TUI's copy of the new vocabulary) — deferred while `crates/`
  churned; unblocked now, not yet done.

### What still needs a terminal
Only the items above that are environment-bound. Nothing in the pane's core function is
terminal-only: every docs/22 row that is terminal-only carries its measured reason.

### Gates
- `make check` (Kampr): **green and stable** — 2376/2376, twice in a row under load. The
  **pre-existing** flaky live-herdr tests are fixed (s39, `6ec0aee5`): seven, each root-caused by
  measurement (timing budgets + two test races, one a missing sync point), no assertion weakened.
- `make check-client`: green. Muffit gate: green.

### The one remaining "done" item
The plan's final bar is a look at the pane on a phone with the four sources (headless Claude,
Claude in a terminal, muffit, the least-capable harness), confirming no empty row, disabled control
or gap. The artboard screenshots stand in for this (four sources × two frames, plus the session
sheet showing the model and mode controls); a real-Pixel pass is optional per the operator.

## Phase 0 — probes

DONE (no turn spent, rows 668–670):
- Zero-cost `get_usage`/`initialize` check: `max` plan, rate limits available, no overage.
- No-turn read control requests (#668): `list_models`, `file_suggestions`, `mcp_status`,
  `list_permission_rules`, `get_context_usage`, `get_session_cost`.
- No-turn state-change control requests (#669): `rename_session`, `stop_task`, `register_repo_root`,
  `rewind_files`, `mcp_reconnect`. (`set_permission_mode` to `bypassPermissions` was read as a
  success in #669 but does not reproduce — struck through there, corrected in #683.)
- Fast-mode billing (#671, from Anthropic's docs): on a subscription plan fast mode is paid from
  usage credits, not the plan's included usage — a money-costing control, a refusal under
  `overage = "refuse"`.

DONE (turn-based, rows 674–684, plan paid and running, `--model haiku`):
- Plan approval (#674): not an `initialize` state; a `can_use_tool` for `ExitPlanMode` with
  `input.plan`; allow reverts to `dontAsk` and the plan runs.
- Images/files on stdin (#675): `image` and `document` base64 blocks accepted; an unprocessable
  image is stripped with an explanatory text block, not a hard failure.
- `priority`/`cancel_async_message` (#676): cancels without a `request_id` answer
  `success {cancelled:false}`; a queued message merges into the running turn.
- Slash commands as input (#677): `/compact`, `/clear`, `/model` are interpreted, not sent to the
  model; `/add-dir` goes to the model as literal text.
- Valid `set_model` value shape (#678): alias or canonical id; the response carries the resolved
  canonical id; omitting `model` resets to the default.
- Controls during a turn (#679): `rename_session`/`file_suggestions`/`mcp_status`/`stop_task` all
  work mid-turn; `rewind_files` is "not enabled".
- `register_repo_root` vs `/add-dir` (#680): not the same; neither extends the readable
  directories in a headless session.
- A subagent's `can_use_tool` (#681): the main agent's shape plus one field, `agent_id`.
- `result.origin` on self-started turns (#682): present on task-notification turns
  (`{kind:"task-notification", producer:"session-task", runId}`), absent on user and slash turns.
- `--allow-dangerously-skip-permissions` (#683): starts the session in `bypassPermissions`;
  without it, the `set_permission_mode` to bypass is refused (`bypass_not_launched`) — the #669
  reading of a success does not reproduce (struck through there).
- Elicitation during a turn (#684): `elicitation/create` arrives as a `control_request
  {subtype:"elicitation"}` with schema and `property_order`; the client's `{action, content}`
  object is what the MCP server receives.

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
launching. **CURRENT STATUS:** Phases 0,1,2a,2b,3,4,5,6,7 are ALL DONE with their review loops
CLOSED (2a=s13, 2b=s21, 3=s23, 4=s12, 5=s19, 6=s20, 7=s25→s26 B1 fix). **Gate-green DONE (s27):**
fmt `b5e11691` + clippy `bf9beefc`; `make check` GREEN (2341 tests). **Phase 8 DONE (s28,
`108a7ce1`)** — the parity pass over docs/22 against the code; it found a **CRITICAL functional
bug**: the headless Claude pane *declares* send/answer but the node routes the pane's `Input`/
`Answer` to the provider's `write_pane` (which bails — a headless pane has no PTY) instead of the
conversation, so composer send and ask-dock answer are drawn but non-functional for headless Claude.
Both prior reviews (s19 money-safety, s25 wire-unchanged) had scopes that missed this. Phase 8 also
named the remaining headless gaps: interrupt (never sends the `interrupt` control request), context
usage (never calls `get_context_usage`), `convo.suggest` (slash/@ completion not handled),
queue/send-now/unqueue (node ignores `when`, never sends `cancel_async_message`), files (dropped on
dispatch), and subagent-live + search/older (not handled). **Phase 5b-1 DONE (ed3751c6, c2bf8799, 7f531e53):** the node's `input`/`answer` handlers now route
a headless Claude pane's verbs to its conversation (looked up by the `c:` pane id) instead of the
provider's `write_pane`, which bails for a pane with no PTY; the node's headless `ClaudeSource` is
reachable from its sessions, and the new `tests/it/headless.rs` integration test proves a composer
send and an ask-dock answer reach the conversation against the mock (a send that bailed to a
terminal would come back as a wire error). **c2bf8799** (s30) fixed the pipe-log reader that
outlived its conversation and pinned the runtime's shutdown (the reason the test sat after passing);
**7f531e53** fixed a regression the first cut introduced: a muffit *session* pane is also
terminal-less, so `!terminal` alone stole the muffit session's send/answer from its REST path — the
`c:` pane id is the discriminator. `make test-fast` green (2151 passed).
**Leak fix (6216d8f5):** the guards and headless tests created keeper-backed Claude sessions under
the shared keeper root, so their state dirs accumulated in `/run/user/1000/kampr/s/` and the node's
`ClaudeProvider` listed them as panes in the operator's client. Both tests now set `KAMPR_KEEP_ROOT`
to a per-test tempdir so sessions live and die with the test.
**Phase 5b-2 DONE (d43feec9, 6b5f0d15):** the headless conversation now handles interrupt (sends
the interrupt control request), context usage (get_context_usage), file/slash completions
(file_suggestions), queue/send-now (input.when + input.unqueue), and file attachments. Wire changes
are additive (input gains optional id/when, new input.unqueue/convo.suggested/convo.context frames).
The headless integration test proves the source declares the full control surface (abilities frame).
**Muffit gate DONE (33e2898):** fmt/build/clippy clean. Test failures are environmental
(`~/.cache/.muffit` data on this machine), not from the conv-sources work.
**Phase 9 probe DONE (s34):** Codex app-server protocol measured (26-row probe log). Key finding:
stdio transport flushes responses one request behind — the source must pipeline.
**Phase 5b-3 DONE (2f851351):** subagent-live stream and paging + search over the session
transcript. The headless conversation now emits `Update::SubLive` for live subagent output and
handles `Older`/`Search` commands against the transcript journal.
**Phase 9 DONE (1ace3aea, 1c2d3df3):** the Codex app-server as the third conversation source
(`crates/kampr-codex/`). NDJSON JSON-RPC 2.0 over stdio, pipelined with `account/read` nudges
(probe row 3: the server flushes responses one request behind). 7 lib tests + 4 integration
tests (turn completes, interrupt, 429→Trouble, create-with-prompt). The mock-stdio.py captures
the measured wire behaviour including the one-behind flush rule. Key bug found: the real server's
notifications carry no `jsonrpc` field — the untagged enum required it and silently dropped
every such line. Fixed with `jsonrpc: Option<String>`, pinned by a regression test.
**Crate graph (8b380f31):** `kampr-codex` added to the ALLOWED list in the guard test.
**Gate status:** `make check` **green and stable** (2376/2376, twice in a row under load). The
pre-existing flaky live-herdr tests are fixed (s39, `6ec0aee5`): seven, each root-caused by
measurement — timing budgets (`the_port_is_bound_before_herdr_is_needed` 5→15 s, the transcript
line 100→300 ms, the booting-harness input 28→100 ms/round) and test races (a missing sync point in
`the_widened_answer...`, a `kampr-docker` OOM-kill the daemon misses 1 in 8 under load). No
assertion weakened; the three that never failed in s39's loaded runs were left untouched. All
conv-sources tests pass: 222 kampr-claude, 476 kampr-node, 7+4 kampr-codex, 10 kampr-testkit guards.
**Next: the gated billing-guards turn probe (after the 5h window reset); the deferred items.**

## Release 0.1.153 + muffit deploy (2026-11-27)

- **kampr**: `conv-sources` (all ten phases + the other agent's pane-modes/OSC-7501/unattended-perms
  work) merged to `main`, pushed. `make check` green (2376/2376). Bumped to **0.1.153**, tagged,
  pushed; `make android-publish` put 0.1.153 on kobup (version.properties committed + pushed).
  `android-attach` runs once the tag's GitHub release exists.
- **muffit**: the other agent's uncommitted work (unattended-permissions raise-to-ask, plus the
  rustfmt pass) committed, `conv-sources` merged to `main`, pushed. Gate green (86 suites, clippy
  clean). Tarball `muffit-0.1.0-7f5ceda61c14` staged **without restarts** on all three daemons:
  this machine, 10.0.0.208 and 10.0.0.229 (previous binary kept for rollback; verified no process
  was restarted). The 10.0.0.208 container image `docker.oldug.com/muffit-server:7f5ceda` +
  `:latest` is built from the same commit and pushed to the registry — the container picks it up on
  its next scheduled `compose pull && up -d`, which nobody has triggered.

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
  (rule-#2 violation). **Fixed (7c164f14, s18):** real dir `temp_dir()/claude-<uid>/<cwd-slug>/<session>/tasks`
  (`rustix::getuid`, no new dep) + `bind()` `facts.bg` exclusion + test rewritten to the real path +
  probe row 673. **Re-reviewed (s20): FIXED** — measured both reds (old dir → test fails; `bind()`
  exclusion removed → test fails), probe log contiguous 1..673, no collateral (3 files). **Phase 6
  review loop CLOSED.**
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
- **s19 — Phase 5 review (money-safety): SOUND** — no test reaches Anthropic (only `127.0.0.1`,
  self-contained mock, launcher refuses under `KAMPR_TEST=1` without a mock); the scrub stops a real
  key *value* crossing under `plan`; guards 3/4/6/7 red-verified (measured), 5/8/9 confirmed;
  lenient reader + request-id matching; no forbidden edits; the node-lib failure is the worktree
  artifact (passes 220 in isolation). **One minor/theoretical gap:** the `ANTHROPIC_BASE_URL`
  blanking exclusion was unconditional, so a production node-env base URL would cross to a `plan`
  child. **Fixed (0c10c6e5):** the exception is now gated on a mock address (`base_url.is_some()`),
  + test `a_stray_base_url_is_blanked_under_plan_without_a_mock`; all 34 + e2e pass. **Phase 5
  review loop CLOSED.** Also fixed the `crate_graph` guard (0fa8cd43) that the Phase 5 commit broke
  (`kampr-node → kampr-claude` edge).
- **s24 — Phase 7 (the pane pump behind the trait): DONE** (`5c899e9c`) — `TerminalClaude:
  Conversation` in `convo.rs` (the pump's journal/preview/hooks → `Update`s through the Phase-2a
  `emit()`), Codex/omp/pi/agy wrapped with a smaller ability set, key-driven steering NOT declared.
- **s25 — Phase 7 review: one BLOCKING defect (B1)** — `has_conversation` now true for terminal
  panes, so `convo.load`/`find`/`sub` on a pane with no transcript went `not_found` → silence
  (two deterministic live tests pinned the old contract and failed; not caught by `test-fast`
  because its nextest profile excludes `live::*`). Wire-unchanged confirmed frame-by-frame;
  abilities PASS; D2 (steer/set/act → silence), D3 (test gap), D4 (pre-existing `reopening` flake).
- **s26 — Phase 7 fix: DONE** (`ce5449cb`, 2 files +40/−9) — `Older`/`Find`/`Sub` emit
  `Update::Missing` when the journal is `None` (restores `not_found`); D2 steer/set/act →
  `Update::Missing`; D3 `lifecycle` assert. Both B1 tests red→green; node 474 (1 = D4 flake);
  clippy 0 new; test-fast 2150. Kept the mass `cargo fmt` reformat OUT (pre-existing drift,
  separate gate-green pass). **Phase 7 review loop CLOSED.**
- **s27 — gate-green (kampr): DONE** — fmt `b5e11691` (rustfmt 1.10.0 / the pinned 1.99.1
  toolchain) + clippy `bf9beefc`; `make check` **GREEN** (2341 tests). The flakes it hit
  (`a_blocked_agent_pane`, the journal `it` set) were environmental — other sessions building on
  the shared tree in parallel — and all pass alone.
- **s28 — Phase 8 (parity pass over docs/22): DONE** (`108a7ce1`, docs only) — checked every row
  against the headless source, the terminal pump, the muffit source and the node's routing. Every
  terminal-only row carries its measured reason; the "Not determined" section is resolved. **Found
  the CRITICAL headless routing bug** (Input/Answer → provider `write_pane`, which bails) plus the
  headless gap list (interrupt, context usage, `convo.suggest`, queue/send-now, files,
  subagent-live, search/older). **Unblocks Phase 5b-1.**
- **s14 — Phase 2b (the client learns the vocabulary, Kotlin): DONE** — committed `80fe0d2a`
  (13 files, +2457/−242, `client/` only). Wire mirror (`Protocol.kt`/`Codec.kt`/`Serializers.kt`):
  all 13 `convo.*` `ServerMsg` + 5 `ClientMsg` + widened `Answer.reply`, field-for-field vs the Rust
  `wire.rs`; hand-written `ReplySerializer`/`TroubleFixSerializer` fixed two real wire-spelling
  mismatches (kotlinx 1.12 `type` discriminator vs the node's serde `{"allow":{…}}`; serde unit
  variants as bare strings). Pane (`ConvoPieces.kt`): trouble banner+fix, task panel (agent→live
  subagent, shell→live-output sheet), session sheet (controls w/ confirm-sentences, actions palette,
  search), facts line (model·context·plan/resets·billed-by), `/`+`@` completion, running-turn stop
  bar, composer send-now/queue. **Every piece early-returns with no room when the source declared
  nothing.** `make check-client` green, 16 codec round-trip tests, screenshots `seam-floor-390.png`
  + `seam-rich-1440.png`. **Deferral:** `kampr-tui` (a Rust crate) not mirrored — the task forbade
  `crates/` while s11 churned it; now unblocked, to do after the reviews. Review in flight (s21).
- **s21 — Phase 2b review**: in a worktree — field-for-field wire parity vs the Rust `wire.rs`
  (the 13+5 frames + widened `Reply` + `TroubleFix`), additive wire both ways (unknown `t`→null, old
  node unaffected by the widened `answer`), the pane draws only what the source declares (the two
  artboards), the confirm-sentence rule, and client-only (kampr-tui the only Rust gap).
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

## Phase 2 — the wire and the client learn the new vocabulary — **2a COMMITTED** (be4c2a9d, +1862/−74; 2089 passed; review s13 = SOUND). **2b COMMITTED** (80fe0d2a, client only; wire mirror field-for-field + pane pieces; `make check-client` green; **review s21 = SOUND** — 13 frames + 6 verbs field-for-field against Rust, `Reply`'s 6 shapes exactly reproduced, additive both ways, board-7 floor + confirm-sentence verified, floor + rich screenshots). **Phase 2 review loop CLOSED.** **Deferral:** `kampr-tui` mirror (Rust crate) not done — to do after the reviews.

## Phase 3 — muffit widened (muffit repo) — DONE (6 commits on conv-sources, wire 3.7→3.09 additive, `cargo test --workspace` 2016 passed; all 7 items: ask tool, live worker, shell tasks, arg-deltas+steer, command catalog, billed-to+rate-limit, prompt-events+typed-REST; item-5 completeness test pins every TUI command in the server catalog). **Review s23 = SOUND** (all 7 items real at implementation level; wire additive via serde(default)+Unknown catch-all, no renames; money-safety clean — `billed_to` is catalog-name+model only, `api_key` never serialized, rate-limit a pure read; collateral clean — unrelated raise-then-refuse work left alone). Three minor cosmetic nits (wire labels skip 308 / "3.10" vs 309; completeness test samples `parse()`→Control, `model.add` in catalog but unverified). **Phase 3 review loop CLOSED.**

## Phase 4 — a keeper for a process that speaks lines — CLOSED (b3b5411c + 278e74b5 + 36622778; kampr-keep 48/48; two reviews passed). Note for Phase 5's pipe consumer: a `Truncated { oldest_seq }` does NOT deliver the `oldest_seq` entry itself — treat it as a hard gap boundary.

## Phase 5 — Claude with no terminal — COMMITTED (cherry-pick `ee203d76`, `kampr-claude` crate + node wiring + `mock.py`): stream-json reader/writer, `launch.rs` scrubbed env + guards 1–5, `billing.rs` guards 3/6–9, `ClaudeSource`/`Provider`/`Herd` (pipe mode), every guard red-tested (34 passed, no test reaches Anthropic). Deviation: guard-2 scrub *blanks* provider names (keeper pipe mode). **Review s19 = SOUND** (one minor/theoretical guard-2 gap — the `ANTHROPIC_BASE_URL` exclusion — **fixed 0c10c6e5** + test; `crate_graph` guard fixed 0fa8cd43; mock-half unit test 73d9fb54). **Phase 5 review loop CLOSED.**

**Billing guards — verified against the operator's real plan login** (manual probe `crates/kampr-claude/examples/plan-probe.rs`, a script a person runs — never part of `make check`, guard 11; the child is launched by the guard's own `launch::argv`/`launch::env` under policy `plan` on the operator's own `~/.claude`, `--model haiku`, Claude Code 2.1.326):
- **Guards 3, 6, 9 — verified against the real login, no turn spent (probe row 685).** `initialize`: `account {subscriptionType: "Claude Pro", apiProvider: "firstParty", …}` with **no `apiKeySource`** — the account the guard-3 check accepts, read live. `get_usage`: `subscription_type "pro"`, `rate_limits_available: true`, `extra_usage: null` (no overage) — the billing facts guard 9 reports (`paid_by: Plan`) and the overage state guard 6 acts on, as of 2026-11-27T04:34Z (5-hour window 94%, 7-day 50%).
- **Guards 2, 4 — the turn-side half NOT yet verified against real Claude.** Both need one haiku turn (guard 2: a key in the *parent* environment blanked by the scrub, the plan login still paying; guard 4: the turn's `system/init` `apiKeySource: "none"`). The 5-hour window was at 94% (above the ~90% line), so per the money discipline the turn was **held**, not spent. The probe is ready: `cargo run -p kampr-claude --example plan-probe -- turn --with-key` — run it once the window has dropped (resets 2026-11-27T07:09:59Z) and log the `system/init` line. The mock coverage stands in the meantime: e2e `the_billing_guards_against_the_mock` (`tests/guards.rs`) plus the unit tests `the_scrubbed_env_keeps_only_the_allow_list_under_plan`, `a_stray_base_url_is_blanked_under_plan_without_a_mock`, `a_key_in_the_environment_is_refused_under_plan_and_named` and the `turn_key_source_ok` cases.
- **Negative cases — mock-tested, all passing** (`cargo test -p kampr-claude`: 55 unit + e2e; `cargo test -p kampr-testkit --test guards`: 10, including the probe-log numbering/citation guards): a key in the environment refused under plan, a logged-out config refused, the bare default (`tokenSource: "none"`) refused, an `isUsingOverage` event mid-turn, the unattended cap, and the launcher refusing under `KAMPR_TEST=1` with no mock (guard 11).

## Phase 6 — Claude in a terminal, the gaps that need no trait — COMMITTED (18c61685, 13 files +1638/−34): trouble parsing (6-kind classify, `StopFailure.error`), 14 hooks installed (PreToolUse/SubagentStart/SubagentStop/`Stop.background_tasks`), fork-aware transcript read, live shell output + live subagent conversations — all through **existing** frames (`convo.trouble`/`convo.task`/`convo.tasks`/`convo.sub`), no wire/client edits. Measured red tests (8/11 trouble, both fork, live shell+subagent). **Review s17: DEFECTS FOUND** (1 blocking: `tasks_tick` wrong shell-task dir; its test passed with the defect) → **fixed 7c164f14** (real dir + `bind()` `facts.bg` exclusion + test to the real path + probe row 673) → **re-reviewed s20: FIXED** (both reds measured). **Phase 6 review loop CLOSED.** Sibling-limits side-channel (guard 10/#658) deferred to Phase 5.

## Phase 7 — the pane pump behind the trait — **DONE** (s24 `5c899e9c`: `TerminalClaude: Conversation` in `convo.rs`, the pump's journal/preview/hooks → `Update`s through the Phase-2a `emit()`; Codex/omp/pi/agy wrapped with a smaller ability set; key-driven steering NOT declared). **Review s25 = one BLOCKING defect (B1)** — `has_conversation` now true for terminal panes, so `convo.load`/`find`/`sub` on a pane with no transcript went from `not_found` to silence (two deterministic live tests pinned the old contract and failed). **Fixed (s26, `ce5449cb`):** the pump's `Older`/`Find`/`Sub` emit `Update::Missing` when the journal is `None` (restores `not_found`), plus D2 (steer/set/act → `not_found`) and D3 (abilities test asserts `lifecycle` empty). Both B1 tests red→green; node suite 474 (1 = pre-existing D4 flake); clippy 0 new; test-fast 2150. **Wire-unchanged confirmed frame-by-frame** (s25). **Phase 7 review loop CLOSED.**

## Phase 8 — nothing left that needs the terminal — not started

## Phase 9 — a third source — **DONE** (1ace3aea; `kampr-codex` crate, 13 files +2752; 7 wire tests + 4 integration tests against the probe's mock, all green; clippy clean, workspace check green). Codex `app-server --stdio` as the seam's third source: the pipelined pump (probe row 3's one-behind flush, nudge per call + 250 ms timer), `CodexConversation` mapping the turn/item/status notifications to `Live`/`Turns`/`State`/`Trouble`/`Notice`, `CodexSource` (create with optional first prompt, open, resumable, end through the herd), and the mock (`research/probe/codex-app-server/mock-stdio.py`) reproducing the measured wire behaviour including threads persisting across process restarts. The stress test's finding: the seam carried everything, including the one edge only a server that asks back exercises — a server→client request becomes an `Ask` and the `Answer` goes out as a best-effort decision (the schema leaves that reply free-form).

## Screenshots + gates — not started