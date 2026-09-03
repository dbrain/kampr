# The harness plane — briefs

What the agent in a pane already writes down, and what Kampr does with it.

Kampr reads a harness two ways today: herdr says *whether* a pane is an agent — by the foreground
process's **name**, not by scraping, which is the half [#355](./03-probe-log.md) corrects — and the
transcript on disk says *what it said*. Both are indirect, and every defect in this brief is
downstream of that. The node runs on the pane's own machine as the pane's own user, and the
harnesses have quietly grown a great deal of structured state beside their transcripts — session
markers, subagent transcripts, per-record metadata — none of which Kampr opens.

Read [`04-wire-protocol.md`](./04-wire-protocol.md) before any of this: every addition below is
additive or it does not ship. Read [`03-probe-log.md`](./03-probe-log.md) for anything about herdr.

The workstreams are independent and can run in parallel except where stated. W0 is not optional.

---

## What is measured, and what is guessed

Everything on this list was measured on the operator's own desktop against Claude Code 2.1.248 and
2.1.250, real transcripts, real live sessions. Anything **not** on this list is a guess and belongs
in W0 before it belongs in code.

| Fact | How | Confidence |
|---|---|---|
| One prompt and the reply to it parse to **53 turns**; `convo::PAGE` is 40, so a page from the newest excludes the prompt | drove `ClaudeAdapter::open_path` + `Journal::poll` over a live transcript | **measured** |
| Claude Code writes **no `isSidechain` records into the main transcript** as of 2.1.248 | `jq 'select(.isSidechain==true)' <transcript> \| wc -l` → 0 | **measured** |
| Subagents live in `<projects>/<slug>/<session-uuid>/subagents/agent-<id>.jsonl`, with a sibling `.meta.json` | listed the directory on two sessions | **measured** |
| That `.meta.json` is `{agentType, description, toolUseId, spawnDepth}` | read four of them | **measured** |
| The parent links to it: the `Agent` tool's `toolUseResult` carries `{agentId, description, isAsync, outputFile, resolvedModel, status, canReadOutputFile}` | `jq` over the transcript | **measured** |
| A subagent transcript uses the **same record grammar** as the main one — `ClaudeParser` reads it unchanged | read the first records of one | **measured** |
| `~/.claude/sessions/<pid>.json` exists from the moment a session opens, minutes before any transcript | compared its `startedAt` against the transcript's `birth` | **measured** |
| It carries `{pid, sessionId, cwd, procStart, version, kind, name, nameSource, status, statusUpdatedAt, messagingSocketPath, peerFeatures}` | read a live one | **measured** |
| The transcript file is created at **first submit**, not at session start; the prompt is line 6 | `stat -c %w` against the session marker's `startedAt` | **measured** |
| A pane's real foreground job is reachable from `/proc/<shell>/task/*/children` even where herdr reports the shell | walked it for every `bash` on the machine; found `claude`, `ssh`, `herdr` under shells whose child shares the shell's pgid | **measured** |
| That shared-pgid case is exactly what `ProcessInfo::command` returns `None` for (`model.rs:158`) | read the code against the measurement above | **measured** |
| The record types in a Claude transcript, and their counts | census over nine transcripts — see W5 | **measured** |
| `node_offline` and `herdr_unavailable` are sent on a reachability change with `pane: None` | read `session.rs`, `reachability_changes` arm | **read** |
| The node has **no git integration at all** | grepped `crates/` | **read** |
| `FileRef` — the whole node half of file retrieval — is complete, tested, and **minted by nothing in `client/`** | grepped both sides | **read** |
| `PaneEntry.cmd` / `.argv` are already on the wire and already rendered by `{argv\|cmd}` in the naming template | read `wire.rs`, `naming.rs`, `Naming.kt` | **read** |
| The notification icon's ink box is **262 × 182 at +125 +244** in its 512 viewport — 35% of the canvas height, 79 units below centre | rasterised the vector with `rsvg-convert` and trimmed | **measured** |
| Which is **12 × 9 px at +6 +11** at mdpi, the size Android actually draws it | same, rendered at 24 px | **measured** |
| Centring it is not enough — the eyes stay 1.6 px and the muzzle 2.8 px, so the sheep reduces to a blob at any offset | rendered the keylined geometry and trimmed | **measured** |
| The tent that replaced it is **470 × 378, centred — 22 × 18 px at mdpi**, and a door taller than about a third of the body turns the glyph into a caret | rendered twelve candidates at 24 px and compared | **measured** |
| The reply box clears a 25 dp selection handle by **116 dp** in portrait, **50 dp** with no tab bar, and **26 dp** on a rotated pane | measured the field's bounds in `ComposerInsetTest`'s harness at three postures | **measured** |
| No `SelectionContainer`, `Popup` or clipping ancestor stands over the composer — the three usual causes of a misplaced handle are all absent | read `BottomChrome.kt`, `ConversationView.kt:267`, and grepped the client | **read** |
| The phone does **not** double the top inset: with `safe.top = 48 dp`, `HerdPortrait`'s title lands at 67 dp — one inset plus its own 16 dp | composed it through `screenInset` and measured the title's bounds | **measured** |
| The **Desktop** breakpoint does: `KamprApp.kt:304` applies `screenInset` and `HerdSidebar` adds `18.dp + safe.top` again | read both call sites | **read** |
| `Theme.Kampr` is the platform `android:Theme.Material.NoActionBar` and sets opaque `statusBarColor`/`navigationBarColor`, which `enableEdgeToEdge()` expects to own | read `themes.xml` against `MainActivity.kt` | **read** |
| `sessions/<pid>.json`'s `status` tracks a live session: `busy \| shell \| idle \| waiting` plus `waitingFor`, rewritten **in place** within ~100 ms of a transition | drift of `statusUpdatedAt` past `startedAt` on live sessions, and a fourth watched from creation ([#344](./03-probe-log.md)) | **measured** |
| Whether the phone's offset is exactly a status bar, and whether the theme is what causes it | — | **guess, W10** |
| `messagingSocketPath` speaks newline-delimited JSON one way, needs no auth on Linux, and `notify_idle` is a real one-shot subscription — but it **cannot report `waiting`**, so it is an accelerator and not the signal | connected to a throwaway session's socket and subscribed, then drove it to a dialog and back ([#346](./03-probe-log.md)) | **measured** |
| Whether Codex and agy have equivalents of any of this | — | **guess, W0** |

---

## The mechanism three of these share

W3, W8 and half of W5 are the same idea wearing three hats: **the node is on the pane's machine, as
the pane's user, and it has been asking herdr questions it could answer itself.**

herdr's `pane.process_info` gives a pid. procfs gives everything below it. That single walk answers
"is this pane an agent" without waiting for a screen-scrape, "what is this pane running" where
herdr's process-group check gives up (#297), and "which session is this" by keying the harness's own
marker file on the pid it finds.

It is not a replacement for herdr and must not become one. herdr owns the herd — panes, tabs,
workspaces, layout, and the fact that a pane exists at all. This is the node reading its *own*
machine about a pane herdr has already named.

**It is local-only, and that is fine.** A node sits beside herdr on each machine by construction, so
the pane and the procfs are always on the same host. A pane reached across the mesh is served by
*its* node doing this walk, not by the hub guessing.

---

## W0 — Probes

**Owns** [`03-probe-log.md`](./03-probe-log.md) rows. **Done — rows #309-#316 are appended.**

They were held unnumbered while a concurrent changeset was in flight, and assigned in one write
once it landed; the log is 316 rows, gap-free.

| Row | What it records |
|---|---|
| **#309** | The per-session directory — `subagents/`, `tool-results/`, `custom-title.json` — and that the main transcript carries no sidechain records any more |
| **#310** | The `agentId` link from the parent's `Agent` result to the subagent file, the `.meta.json` schema, and the `queue-operation` a completion arrives as |
| **#311** | `~/.claude/sessions/<pid>.json`: full field set, and that it precedes the transcript by 2 min 42 s. `status` and `messagingSocketPath` are recorded **as unmeasured**, which is the point of the row |
| **#312** | The record-type census, with counts, and the standing warning that it is Claude only |
| **#313** | The procfs walk naming the job in exactly the case #297 gives up on — and the three cases still unmeasured, staleness first |
| **#314** | 53 turns to one exchange against a page of 40, and that this is *not* the `fresh`-page defect |
| **#315** | The notification icon's geometry, before and after, and the caret threshold found across twelve candidates |
| **#316** | The composer's handle clearance at three postures, the phone's correctly single-counted inset, and the Desktop double-count |

**Still open, and deliberately written down as open** rather than left to be rediscovered:

- ~~Whether `sessions/<pid>.json`'s `status` tracks a live session.~~ **Answered ([#344](./03-probe-log.md)): it tracks.**
  W3 therefore *replaces* herdr's `agent_status` for a Claude pane rather than supplementing it,
  and does so at no extra cost — the marker was already being read once per pane per rebuild.
- ~~What `messagingSocketPath` speaks.~~ **Answered ([#346](./03-probe-log.md))**, and the door was
  worth opening carefully: it works with no configuration, and it still is not the answer, because
  the idle feed is wired off `idle|shell` and `busy` and a session blocked on input fires nothing.
  The marker covers what it cannot, and is not behind a server-side kill switch ([#347](./03-probe-log.md)).
- The same census against a Codex rollout and an agy transcript. W5's shape depends on it.
- The procfs walk against a backgrounded job, a multi-member pipeline, and — the one that matters —
  a job that has already exited.
- Why a real phone's selection handles land off screen. No JVM harness can see a `Popup`.

---

## W1 — The conversation stops losing its own question

**Owns** `crates/kampr-node/src/convo.rs`, `ConversationView.kt`.

**The defect, measured.** `PAGE = 40` counts *turns*, and every tool call is a turn — each harness
writes one call per record and `ClaudeParser` carries that through. One prompt and one working reply
in this repo parse to **53 turns**. So a page taken from the newest turn opens thirteen turns *into*
the reply, and the prompt that caused it is not in it. On a brand-new session with a single
question, on a view pinned to its own end, that reads exactly as reported: the agent working, and
nothing to say what it was asked.

`Reply` in `Exchange.kt` already anticipates the shape — *"a transcript paged backwards can open in
the middle of one, so a reply with nothing in front of it is a reply, not a broken exchange"* — and
renders it headless. Correct for a genuinely older page. Wrong as the **first** thing a reader sees.

**The rule: a page never opens mid-reply.** `page_before` extends backwards from its 40 to include
the `Role::User` turn that opens the reply it landed in, and everything between. `PAGE` becomes a
floor rather than a ceiling. A ceiling is still needed — a reply with four hundred tool calls must
not arrive whole on a phone — so the extension is bounded, and when the bound is hit the page says
so rather than silently opening headless again.

This is a node-side rule and it fixes every installed client with no wire change, which is why it
is first.

**Then the operator's own ask: page the whole history, lazily.** Nothing caps how far back
`convo.load` can walk today — the `TurnStore` holds every turn the file has — so this is an
affordance problem, not a capacity one:

- The scroll trigger (`first <= 1`) cannot fire while the view is following its own end, which is
  every moment of a working turn. Give the reader a **visible** "older" control at the head of the
  list rather than only a scroll position that a working pane keeps taking away from them.
- `asked` at `ConversationView.kt:143` is `remember { }` with no key, so it survives a pane switch
  and can swallow the first `convo.load` on the pane switched *to*. Key it on `pane.id`.
- Consider paging by exchange from the client too: "load the previous exchange" is the unit a reader
  asks in, and `Reply` is already that unit on this side.

**Not to be confused with the other one.** `held` lives in the per-socket session
(`session.rs`, `convo::held()`), so a reconnect or a node restart genuinely does send a replacing
`fresh` page and the client genuinely does `turns.clear()` (`KamprStore.kt:197`). That is real, it
is worth fixing with an opaque `path_key` on the page so a client holding the same transcript merges
instead of clearing — but it is **not** what the report was about, and fixing it alone would have
changed nothing.

**Done when** a session with one prompt and a two-hundred-call reply opens showing the prompt, and a
test that restores `PAGE = 40` as a hard ceiling fails.

---

## W2 — Subagents

**Owns** `crates/kampr-journal/src/claude/`, a new nested shape on the wire, `ConversationView`.

**This is the largest single gap and it is almost free.** Claude Code stopped writing sidechain
records into the main transcript; `ClaudeParser::ingest`'s `is_sidechain` skip is now dead code
against current versions. The content moved to `<session>/subagents/agent-<id>.jsonl`, in the same
record grammar `ClaudeParser` already reads. Nothing needs a new parser.

The link is exact: an `Agent` tool's `toolUseResult` carries `agentId`, and `agentId` names the
file. `agent-<id>.meta.json` beside it gives `agentType` and `description` — the card's header,
with nothing parsed.

**But the result is not the only way in, and against Claude Code 2.1.252 it is not always a way in
at all.** Measured, and the reason the ingest has two mint points rather than one:

| Measured against 2.1.252 | Number |
|---|---|
| `run_in_background: true` writes its result **at launch** — `status: "async_launched"`, `agentId`, `outputFile` | 12–585 ms after its own `tool_use`; completion arrives ~117 s later as a separate `queue-operation` |
| Of the operator's whole archive, launches that are `async_launched` | 176 of 177 |
| `run_in_background: false` (new in 2.1.252) writes **no result at all**, and its `tool_use` input carries no `agentId` | for 65–146 s |
| `agent-<id>.meta.json` is written at launch, carrying `{agentType, description, toolUseId, spawnDepth}` | +12 to +20 ms after the `tool_use` record; 4 of 4 synchronous launches, distinct `toolUseId` per agent under three concurrent launches |
| The subagent's own `.jsonl` appears after the record that names it, and grows continuously | +3 to +5 ms; 8 595 → 25 842 bytes over 85 s |
| **The result record beats its own transcript to disk** | +0.101 s and +0.777 s, two runs |
| Metas born late — all of them asynchronous launches, none synchronous; never rewritten (inode, size, mtime, ctime unchanged for a whole agent life) | 15% of the archive, 229 s to 11 030 s. No mechanism established |

Three consequences, and all three are in the code:

1. **A handle may not be conditioned on the file being there.** `settle` runs once per
   `tool_use_id`, from one call site, with no retry — so a poll landing in the sub-second window
   between the result and the transcript used to drop the card for the life of the session.
   `SubRef` is a *name*; it is resolved through `TranscriptRoot::contain` at **open** time, which
   canonicalises and so refuses until the file exists. A transcript 100 ms late and one that is
   never coming are the same thing at mint time, and only the second is worth refusing — so the
   card is minted either way and the refusal happens where it can be answered honestly.
2. **A synchronous launch is minted from `agent-<id>.meta.json`'s own `toolUseId`**, at the
   `tool_use` record, because there is nothing else for two minutes and those two minutes are the
   whole of the time somebody would want to watch. Matching is on `toolUseId` and **never** on
   file-creation order: three launches in flight at once each wrote their own meta, and ordering
   would be a guess dressed as a rule.
3. **The meta stays non-load-bearing for the asynchronous path.** A 15% tail of async metas is born
   minutes to hours late; one that is not there yields no card at the call and `agentId` mints one
   at the result exactly as before. One `tool_use_id` is one card — whichever end minted it.

The shape:

- The tool card for an `Agent` call carries the agent's type and description instead of the generic
  summary, and is **openable**. That is the operator's ask: *see what the agent is doing by
  selecting it.*
- Opening it opens the subagent's transcript as a conversation in its own right — the same
  `ConversationView`, one level down, with a way back. `spawnDepth` says a subagent can spawn its
  own, so the shape must nest rather than assume one level.
- It is **live**. The file is appended to while the agent runs, and `FileJournal` already tails a
  growing file. A running subagent's card shows its latest step; a finished one shows its result.
  Live of a sub already open was the easy half; the *way in* is the half that was measured wrong,
  and the table above is why.
- `status` off the parent's `toolUseResult` (`async_launched`, and whatever the completion
  notification carries) is what the card's state comes from, not a guess from the file's mtime.

**On the wire.** A subagent is a conversation, so the honest addition is a *scope* on the existing
conversation verbs rather than a new turn type: `convo.load` and `convo.turn` gain an optional
sub-conversation handle, and a client that does not send one gets exactly what it gets today. Do
**not** inline subagent turns into the parent's turn list — an installed phone would render them as
the parent's own reply, which is a lie about who said what.

**The cheap half first.** Even with no nesting at all, reading `.meta.json` to put *"Explore — Map
the manage op end-to-end path"* on the card, instead of a truncated prompt, is most of the
legibility for a morning's work. Ship that, then nest.

**Depends on** W0's subagent row.

### W2b — What is running *now*, in one place

**Shipped.** The card above is right and it was not enough: it appears in the turn that launched the
agent, which a reader only reaches by scrolling back to the moment of the launch. The operator, on
0.1.49: *"i was expecting some representation in a static location ... because sometimes claude
leaves shells open forever and 'working' can mean nothing but 'a shell was left running'"*.

**`agent_status` cannot express this and should not be made to.** A pane reports `working` while
anything at all is outstanding. That is correct and it is one word for two situations the operator
has to tell apart: an agent thinking, and a shell somebody forgot about.

What closes a launch is measured and is not what it looks like ([#418](./03-probe-log.md)):

| Measured | Consequence |
|---|---|
| A background `Bash` writes its `tool_result` **at launch**, in 300–400 ms, carrying `backgroundTaskId` and an empty `stdout` | An outstanding `tool_use` finds **nothing** — 333 calls, 333 results, 24 launches among them |
| An asynchronous `Agent` writes `status: "async_launched"` | Same: an acknowledgement, not an ending |
| The ending is a `queue-operation` `enqueue` whose content is a `<task-notification>` naming `<tool-use-id>` and a `<status>` — `completed`, `killed`, `failed` | This is the one signal that closes both kinds |
| A **synchronous** `Agent` never gets a notification | Closed by its own real result |
| The harness's own `<note>`: a task may notify more than once, because an agent can be resumed | A second launch of a call id **reopens** it |

`crates/kampr-journal/src/claude/running.rs` folds it; `Facets.running` carries it, additively;
`RunningStrip` draws it pinned above the reply box with a per-second stopwatch off `since` — an age
would say `2m` and then not move for forty-nine seconds, which is the frozen-counter complaint
[#285](./03-probe-log.md) already earned once. It is a **read**: nothing on it presses anything, and
the way into a launched conversation is still the card the turn carries.

**Not done, and deliberately left:** the herd list still says only `working`. Putting the count on
`PaneEntry` would let a reader tell a busy machine from one with a stale shell on it *without
opening the pane*, and that is a wire addition and a separate piece of work.

---

## W2c — A question the operator can actually answer

**Shipped.** Separate from the subagent work and reported alongside it: *"we get options to select
from with no context around them and the context is the most important part"*.

**The transcript is still not a source, and that was re-measured rather than assumed.** #42 said
Claude writes nothing until the question is answered; against 2.1.258, driving a real
`AskUserQuestion` and polling for 60 s, that still holds — 0 records on disk while the dialog stands,
and the `tool_use` and its result land together the moment it is answered
([#421](./03-probe-log.md)). So `source: "screen"` stays honest and the screen is where this comes
from.

**But the screen was being read for a fifth of what it says.** The dialog draws its own title, and
under every option the harness's own `description`; Kampr published five bare labels. `pending` now
carries `header` and per-option `detail`, both additive, and the strip draws each option as a card
with its description rather than a row of chips — a paragraph cannot be laid beside three others on
a phone.

**And a multiple-answer question is a different dialog with a different keystroke grammar**, which
is the part that would have shipped as a lie:

| Measured (#421) | Consequence |
|---|---|
| It draws `[ ]` against each option, and `←  ☐ Test suites  ✔ Submit  →` above them | The checkboxes are the only thing on the screen that says which kind it is |
| A digit **toggles**; two digits left the tool uncompleted | A chip that reads as an answer is not one |
| `\r` toggles the **focused row** and still does not commit | Enter is not the submit key here, whatever the footer says |
| Right-arrow then Enter completes it, with the ticked answers in the transcript | The commit is a sequence, and it belongs in the node |

So `pending` carries `multi` and per-option `chosen`, the card says a press is a tick and offers a
Send that counts what is ticked, and `answer.submit` is a frame of its own — a flag on `answer`
would mean reinterpreting a required field. The commit sequence sits beside the submit-key table in
`session.rs` because it is a measurement, and a harness nobody has raised one on is refused rather
than sent a guess into a live dialog.

**Not done:** the permission prompt is unchanged and has nothing to change — it draws no title, no
descriptions and no checkboxes, which the tests assert so that it stays that way.

---

## W3 — A pane is an agent the moment the agent opens

**Owns** `crates/kampr-core/src/herdr_provider.rs`, `crates/kampr-journal/src/claude/mod.rs`.

Two gates stand between opening `claude` and seeing a conversation, and they are usually blamed on
each other.

1. `has_conversation` means *a transcript file resolves*. The file is not created until the first
   prompt is submitted — measured: session opened 13:00:13, transcript born 13:02:55, the prompt on
   line 6. So `PaneScreen`'s `talks` is false for the whole of that gap and the view falls back to
   the terminal.
2. Before that, `HerdrProvider`'s sweep only looks a pane up at all when **herdr** has already
   scraped an agent out of it (`wanted.get(&pane_id)`). No scrape, no harness, no conversation,
   whatever is on disk.

The marker file settles both. `~/.claude/sessions/<pid>.json` exists from session start, is removed
on exit, and carries `sessionId`, `cwd`, `procStart` and a name. `ClaudeAdapter::locate_by_process`
already reads it — including the `procStart` check that stops a recycled pid being believed — but
only for panes herdr has already labelled.

**Invert it.** Take every pid in the pane's pipeline from `process_info`, intersect with the pids in
the marker directory, and the answer is exact, immediate, and independent of herdr. It survives
ble.sh, where `process_info` names only `bash` (#297), because it matches on **pid**, not on name.

Then:

- A pane with a marker and no transcript yet is an agent pane with an **empty** conversation, not a
  pane with no conversation. `has_conversation` needs a third state, or the client needs to stop
  treating "no turns yet" as "not an agent". Say which in the code; do not let the two answers drift.
- The marker's `status` **is** a better `agent_status` than herdr's screen answer, and is now what a
  pane reports: `harness_status` in `kampr-node/src/state.rs`. Ungated by [#344](./03-probe-log.md),
  and sharpened by [#360](./03-probe-log.md) — herdr's evidence buffer never backfills, so it can
  publish a confident `idle` indefinitely at a pane that is working.
- `name`/`nameSource` feeds W5's naming.

**Generalise, do not special-case.** This is a `JournalAdapter` capability — "which session is this
pid on, and is it live" — that Claude fills from its marker directory and the other adapters fill
however they can, or not at all. Codex must not become a second-class harness because Claude grew a
convenient file.

**Depends on** W0's marker row.

---

## W4 — An offline node is a fact about the herd, not an alert on the pane

**Owns** `crates/kampr-node/src/session.rs` (the `reachability_changes` arm), `KamprApp.kt`.

On a peer going unreachable the node sends two errors with `pane: None`:
`HerdrUnavailable` with the node's `detail`, and `NodeOffline`. `pane: None` lands them in
`KamprStore._failure`, which is a single value rendered by the global `ErrorStrip` floating over
whatever screen is open. So a node the operator is not looking at interrupts a pane on a different
node, on a phone.

Everything needed to say it quietly already exists: `nodes[].online` and `nodes[].detail` render as
the offline dot and its reason on the herd screen (`HerdPieces.kt:207`), and `PaneScreen`'s
`streamFault` already puts `info.detail` on the pane surface itself.

**The rule the operator gave: a disconnection is loud only when it is the thing they are using.**

- Stop broadcasting. The herd patch already carries `online: false` and `detail`; the strip adds
  nothing an installed client cannot already draw. Removing a send is safe for old clients — they
  show the herd state instead, which is what they should have shown.
- If a loud form is still wanted for the pane in hand, send it **with `pane` set**, for panes this
  socket is actually watching on that node. `ServerMsg.Failure` has carried `pane: String?` all
  along and the client ignores it for display; route pane-scoped failures to the pane surface and
  leave the global strip for things with no pane — auth, refusals, the socket itself.

The client's own disconnection stays exactly as loud as it is. That is `ConnectionStatus.Offline`
and a different code path.

**Smallest workstream here.** Two files, and a test named for the defect:
`a_node_going_offline_elsewhere_does_not_interrupt_the_pane_in_hand`.

---

## W5 — The record harvest, and it has to be per-harness

**Owns** `crates/kampr-journal/src/{adapter,model}.rs`, all three adapters, one additive wire frame.

`ClaudeParser::ingest` handles `type` in `{assistant, user}` and silently drops the rest. Census
across nine transcripts in one project:

| Record / attachment | Count | What it is |
|---|---|---|
| `ai-title`, `custom-title.json` | 38 / — | A generated conversation **title**, and a manual override |
| `agent-name` | 9 | The session's own name |
| `system` / `turn_duration` | 413 | `durationMs` + `messageCount` per turn — real timings, not inferred |
| `system` / `compact_boundary` | 7 | `preTokens`, `postTokens`, `cumulativeDroppedTokens`, `trigger` |
| `queue-operation` | 16 | **Queued prompts**, enqueue/remove with a `reason` |
| attachment `queued_command` | 294 | **The prompt itself**, and for one absorbed mid-turn it is the *only* record of it — there is no `user` record at all ([#462](./03-probe-log.md)). `origin` is `{"kind":"human"}` on the 64 a person typed and null on the 230 the harness queues to itself ([#463](./03-probe-log.md)) |
| `permission-mode`, `mode` | 37 each | plan / bypassPermissions / normal |
| `last-prompt` | 36 | Current prompt + `leafUuid` |
| `file-history-snapshot` / `-delta` | 5 / 5 | Per-file backup history keyed by message — see W6 |
| `cost-state` | — | Token and cost state |
| attachment `plan_mode`, `plan_mode_exit`, `plan_file_reference` | 8 | Plan mode, with the plan file's path |
| attachment `invoked_skills`, `skill_listing` | 15 | Which skills ran |
| attachment `command_permissions`, `hook_system_message` | 8 | Hooks, and permission decisions |
| `toolDenialKind`, `userFeedback` | 2 | A denied tool and what the operator said about it |
| `effort` on assistant records | 262 | Reasoning effort per message |
| attachment `total_tokens_reminder` | 3605 | Noise. Ignore it. |

**The trap is shipping this as Claude-shaped fields.** Kampr serves three harnesses and the wire is
additive for ever; a `convo.ai_title` field is a promise that the other two will have one. So:

- The harvest is a **normalised, wholly-optional facet set** on the adapter — a session title, a
  mode, a queued-prompt list, per-turn duration, a compaction marker, a denial. Claude fills them
  from the table above. Codex and agy fill what they have, which W0 will say.
- It reaches the client as one additive frame beside `convo`, not as new fields inside `turn` —
  because most of it is about the *session*, not about a turn, and an installed client that ignores
  the frame behaves exactly as it does now.
- Every facet is absent by default and a client draws nothing for what it does not get. A harness
  with no titles must not render an empty title bar.

**Order by what an operator sees from the sidebar.** Title first (it feeds W8), then per-turn
duration, then queued prompts, then mode. Denials and skills are a power-user seam and can wait.

**Naming, and the operator's rule: automatic only where nothing manual exists.** The naming template
is already first-hit-wins and already shared between Rust and Kotlin, held together by
`crates/kampr-core/tests/fixtures/naming-cases.json` and `NamingParityTest`. So this is one new
field in one template, on both sides, with a fixture row:

```
{label|title|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}
```

`label` is herdr's, which is what the operator set by hand, and it still wins. `title` is
`custom-title.json` before `ai-title` before the marker's `name` — manual before generated at every
level. Change both halves and the fixture in one commit or the sidebar and the CLI disagree.

**Depends on** W0's per-harness census.

---

## W6 — The file plane

**Owns** `client/` (the whole of it — the node half is done), and a decision the operator has
already made.

`FileRef` in `crates/kampr-journal/src/attach.rs` is complete: a `file`-tagged attachment id, `~/`
expansion that refuses `~user`, absolute-only, stat-before-open so a fifo cannot hang the node, an
8 MiB ceiling checked before allocation, and one uniform 404 for every refusal so the route cannot
be used to map the filesystem by response code. `kampr-node/src/attach.rs::serve_file` serves it,
gated on a device that may send input. Ten tests across `attachment.rs` and `attachment_mesh.rs`.

**Nothing in `client/` mints one.** The feature is built and unreachable — dead code by this
project's own definition.

The client half:

- Recognise a path and make it a target. `md/Urls.kt` was written for this — *"so a second kind of
  target can be recognised beside this one"*. Start with paths the **node** derived and a reader
  cannot dispute: a tool card's `summary`, which `summarise()` already fills from `file_path`/`path`
  for every Read/Edit/Write, and `Block::Diff { path }`. Free-text path detection is a guess about
  prose and belongs second, if at all.
- Handle the kind. An image already has a viewer (`ImageViewer`). Text gets the code viewer below.
  Anything else is a download, which `AttachmentCard` already offers for an unknown `kind`.
- The **terminal** surface should mint them too. The operator's framing is right: these are their
  own machines and the node runs as them, so a path on the screen is a thing they can already `cat`.

**The code viewer, and its three sources of diff.**

1. *From the conversation* is already on the wire and rendered. `Block::Diff` is rebuilt from
   `structuredPatch` (`claude/record.rs::unified_patch`) and `DiffView.kt` draws it. Done.
2. *From HEAD* needs git, and **the node has none** — the branch in the sidebar is herdr's. This is
   a new node capability, `git diff HEAD -- <path>` in the pane's cwd, and it must be priced as new
   surface: a subprocess, a repo that may not exist, a path outside the work tree, and a diff large
   enough to need the same ceiling `FileRef` has.
3. *As of a point in the conversation*, which nothing has considered: Claude writes
   `file-history-snapshot` and `file-history-delta` with `trackingPath` and `backup`, its own
   per-file undo history keyed by `messageId`. That is "what did this file look like before that
   turn" without git at all. Speculative; a W0 row before a line of code.

**The honest flag, and it is the operator's call to make — they have made it.** Treating the herd as
a shared filesystem is a real widening. Today the route hands back *a file whose path you already
named*; it refuses directories and gives one indistinguishable 404 for missing, unreadable and
escaped, which is what stops it being a filesystem-mapping oracle. A browser deliberately gives that
property up. That is a defensible trade on machines the operator owns, where kampr runs as them and
a readonly device is a separate role — but it is a change to
[`08-threat-model.md`](./08-threat-model.md) §5, not an implementation detail, and it should land as
one.

**And the trap that has already been sprung once.** #304: `att.fetch` with a file id was the one
verb reading any path on the host that did **not** re-read the device row at dispatch, so a hub
demoted out of process was served a whole file before its `role` frame arrived. It is allowed
*because* it is equivalent to typing. Anything added here goes in the same arm as `input`,
`manage` and `answer`, with a test that demotes through a second store.

---

## W7 — Paste

**Owns** one new client→node message, and a scratch directory on the node.

The client can send `input.text`, `input.b64`, `input.keys` and nothing else, so there is no way to
put a picture in front of an agent from a phone. The operator's diagnosis is right and it is the
whole design: over ssh, an agent reads a **local path** perfectly; it is the terminal's own
image-paste protocol that dies.

So: the client uploads bytes, the node writes them under a scratch directory it owns, and the node
types the *path* into the pane as ordinary input. The agent reads a file, which it is extremely good
at, and nothing has to understand a terminal graphics protocol.

**This is the first thing in this brief that hands a remote client a write primitive on a node**, so
it is last, and it does not go in on a shrug:

- Same gate as typing, in the same dispatch-time arm (#304 again) — and it *is* typing, in the end.
- The node owns the path entirely. The client names a file, never a location. A scratch directory
  under the node's own state, a generated name, an extension derived from sniffed content and not
  from a client-supplied one.
- A ceiling and a total budget, and a sweep that removes what it wrote. A pane that never reads its
  paste must not leave the file there for ever.
- Refuse rather than truncate, exactly as `FileRef::fetch` does when a file grows under it: a body
  short of what it claims is the shape of a wrong answer that looks right.

**Depends on** W6's threat-model change landing first, because they are the same argument.

---

## W8 — The sidebar says what the pane is doing

**Owns** `crates/kampr-core/src/herdr_provider.rs`. **No wire change, no client change.**

`PaneEntry.cmd` and `.argv` have been on the wire all along, and the default naming template already
renders them: `{label|title|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}`. A pane running `top`
*should* already read `kampr (top)`.

It does not, and the reason is written down. `ProcessInfo::command` returns `None` when the
foreground process group equals the shell's (`model.rs:158`), which is exactly what ble.sh does to
every interactive shell on the operator's machine (#297) — so `cmd` is blank on precisely the panes
the operator wants named.

Measured on that machine: walking `/proc/<shell>/task/*/children` finds `claude`, `ssh` and `herdr`
under shells whose child shares the shell's pgid — the case `command()` gives up on. The node is on
that machine, as that user. It can simply look.

- Fill `cmd`/`argv` from the procfs walk **when herdr's answer is the shell or is absent**, and from
  herdr otherwise. herdr stays the source of truth where it has one.
- A pane genuinely at its prompt must still report nothing. The `[…]` group exists so `kampr ()` is
  never rendered, and a walk that names a shell as a job would defeat it.
- **The failure to guard is staleness**, not absence. Naming a job that exited is worse than naming
  none, and it is what a cached walk will do. Re-walk on the sweep, never hold.

Same walk as W3, one sweep, one cost.

**Depends on** W0's procfs row — specifically the pipeline and backgrounded-job cases, which are not
yet measured.

---

## W9 — The notification icon is a dot, and it is measurable

**Owns** `client/androidApp/src/main/res/drawable/ic_kampr_notification.xml`, and one instrumented
test.

Not part of the harness plane, and on this list because it was reported with the rest. **The
drawable is done and in the tree**; what is left here is the test that stops it regressing.

**Nothing ever checked the one thing the file already knew about itself.** Its comment said *"It has
to read at 24dp in a status bar"* — the right constraint, written down, never verified. Rasterised
out of the vector:

| | ink box in the 512 viewport | at mdpi, the real 24 dp status bar |
|---|---|---|
| the sheep, as it was | **262 × 182 at +125 +244** | **12 × 9 px at +6 +11** |
| the sheep, on the keyline | 450 × 314, centred | 22 × 16 px at +1 +4 |
| the tent, shipped | 470 × 378, centred | 22 × 18 px at +1 +3 |

So the artwork occupies 51% of the canvas width and **35% of its height**, and its ink centre sits at
y 335 against a canvas centre of 256 — **79 units, 15%, below centre**. At the size Android actually
draws it that is a nine-row blob in the bottom half of a twenty-four-row badge. It is a dot because
it is a dot.

The interior is worse than the outline. The eyes are 34 × 36 viewport units and the muzzle 60 × 46,
which at that scale are **1.6 × 1.7 px and 2.8 × 2.2 px** — so the knockouts the comment describes
cannot exist at the only size that matters, and the silhouette fills back in as a blob.

**Centring it was not enough, and that is the finding.** Scaling the sheep 1.717× about its ink
centre does put it on the keyline — 450 × 314, 22 × 16 px at mdpi — but the eyes are still 1.6 px
and the muzzle 2.8 px, so what arrives is a legible *blob* rather than an illegible one. The sheep
is lifted from the launcher icon, which is an illustrated scene, and it does not survive the
reduction at any offset.

**So the artwork is now the tent from that same scene** — the other half of the illustration, and a
shape that is nothing but silhouette. Shipped: ink box 470 × 378 of the 512 viewport, centred on
both axes, **22 × 18 px at mdpi**. Drawn as one `evenOdd` path — a flared body with a small doorway
knocked out at the base — because a door reaching more than about a third of the height splits the
silhouette into two legs and the glyph reads as a caret rather than a tent. That was measured across
twelve candidates, not guessed, and it is the constraint a future editor will otherwise re-discover.

**Then make it impossible to regress, because that is the whole lesson here.** A rule nobody can
check is how this happened. `androidApp` already has an instrumented test asserting the notification
against a real `NotificationManager`, and the Makefile already has a `connectedAndroidTest` target —
so inflate the drawable, rasterise it at 24 dp, and assert the ink box mechanically:
`the_notification_icon_fills_its_keyline_and_is_centred`. It fails the moment the old geometry is
restored, which is what rule 2 asks of a fix.

Two things to confirm while it is open, neither yet measured:

- **mdpi is the floor, not the whole story.** Android draws the small icon at 24 dp, so the pixel
  count rises with density and the numbers above are the worst case. Check the shade as well as the
  status bar.
- **Android 12+ draws the small icon inside a circular, tinted badge.** A silhouette that fills a
  square keyline can foul the inscribed circle. Look at it on a real device at the target API before
  calling the geometry settled.

---

## W10 — The window sits a status bar away from where the OS thinks it is

**Owns** `client/androidApp/src/main/res/values/themes.xml`, `MainActivity.kt`,
`HerdScreen.kt`, and a reproduction this repo does not yet have.

Reported from a phone: selecting text draws the drag handles off screen — **in the transcript as
well as the reply box** — as though the whole screen were offset from where the system thinks it
is, by roughly the height of the status bar. Reported alongside it, and almost certainly the same
fact seen from the front: the gap between the status bar and the herd header is larger than it
needs to be. Other apps on the same device are fine.

Two symptoms, one offset, and the second one is the tractable end of it.

**Eliminated, by reading.** The three usual causes of a misplaced handle are all absent, which is
what sends this upward into the window rather than sideways into the composer:

- **Not a `SelectionContainer` conflict.** `screenSelects` (`BottomChrome.kt`) returns false for
  `Screen.Pane`, so no container wraps the pane; the transcript's own
  `SelectionContainer(Modifier.weight(1f))` (`ConversationView.kt:267`) stops at the transcript, and
  the composer is its sibling below it, outside.
- **Not a `Popup`/`Dialog` ancestor.** The only one in the client is `SetupScreen.kt:211`.
- **Not a clipping ancestor** — and a handle is its own window, so clipping could not hide one.

**Eliminated, by measurement.** The obvious theory is that the app pays the top inset twice. On a
phone it does not: composed through `screenInset` with `safe.top = 48 dp`, `HerdPortrait`'s title
lands at **67 dp** — one inset plus the header's own 16 dp, which is exactly single-counted.
`HerdPortrait` never adds `safe.top`, and that is correct.

**Found while looking, and real: the Desktop breakpoint *does* double it.** `KamprApp.kt:304`
applies `screenInset(state.screen)` to the whole column, and `HerdSidebar` inside it adds
`18.dp + safe.top` again (`HerdScreen.kt:227`). On every non-pane screen that is two status bars of
padding above the sidebar title. It is not the phone report — it cannot be, the phone does not go
through that path — but it is a defect, it is one line, and it is the same mistake the report is
about, so it belongs here.

**So the phone's offset is not in the padding arithmetic**, which is the useful conclusion: the
layout is single-counting correctly and the content still lands low, so what disagrees is the
*window*, not the column inside it. That also explains the handles, which the layout has no say
over at all: a `Popup` is a real window positioned in screen coordinates, so a composition whose
origin the system and Compose disagree about puts every handle exactly that far out.

**The first thing to try, because it is cheap and it fits.** `Theme.Kampr` is the *platform*
`android:Theme.Material.NoActionBar` and sets `android:statusBarColor` and
`android:navigationBarColor` to an opaque `#FF0E0F13` — while `MainActivity` calls
`enableEdgeToEdge()`, which expects to own both. A window that is not as edge-to-edge as the
insets say it is would produce exactly this pair of symptoms. Take the two colours out of the
theme, or move to a theme `enableEdgeToEdge` is written against, and measure again.

**What would settle it, on a device:**

- Is the offset *exactly* the status bar height, or the status bar plus something? Exactly is the
  window; not-exactly is arithmetic after all.
- Does it survive hiding the status bar? An offset that vanishes with the bar names its own cause.
- What does `WindowInsets.systemBars` actually report against the real bar height — the same, zero,
  or twice?

**Then build the repro the JVM cannot give.** Handles are `Popup` windows and `runComposeUiTest`
creates none, so this is an instrumented test — the same place W9's icon check belongs, on the same
`connectedAndroidTest` target. Assert a known point's position on screen against where the
composition thinks it is; that is the offset, in one number, and it fails before the fix.

**And while it is open**, the clearance below the reply box is separately too tight. Measured in
`ComposerInsetTest`'s harness against the 25 dp a Compose handle hangs below its line:

| posture | clearance under the field | verdict |
|---|---|---|
| portrait, tab bar | 116 dp | fine |
| portrait, no tab bar | 50 dp | fine |
| **rotated pane** | **26 dp** | **1 dp of slack** |

A larger font scale, a taller keyboard or a vendor handle drawable a fraction bigger and it is under
the keys — on the posture that already needed its own test
(`aRotatedPaneHasNothingUnderTheReplyBoxButTheReplyBox`). The left edge is the same shape: the field
starts at 28 dp and the start handle hangs 25 dp to its left, so selecting from the first character
leaves 3 dp. Reserve the handle rather than discover it. This is not the reported defect and must
not be mistaken for a fix for it.

---

## What was built

Recorded here rather than in a commit message because the brief is what a reader arrives at, and a
plan with no account of itself is how "what already exists" tables go stale — which is exactly what
[`05-agent-briefs.md`](./05-agent-briefs.md) warns about at the front of this repo.

| | State | Proof |
|---|---|---|
| **W0** | Done | Rows **#309-#320**; the log is 320 rows, gap-free. #320 corrects #312's own parenthetical |
| **W1** | Done | `page_before` reaches back to the question. Red first, at `a12` with the prompt gone |
| **W2** | Done | `Block::Sub`, `Registry::open_sub`, the `convo.sub` verb, `SubCard` and a nesting `SubConversationView`. A sub page never reaches `pane.turns` — mutation-checked, 3 tests fail if it does. **Reopened and closed again against 2.1.252**: the card is now minted at the `tool_use` from `agent-<id>.meta.json`'s `toolUseId` as well as at the result from `agentId`, and neither mint asks whether the transcript is on disk yet — restoring the `is_file()` gate fails 3 tests, removing the call-time mint fails 2, removing the one-card-per-call guard fails 3 |
| **W3** | Done | Marker matched on pid, wired into `convo::identity`. Fixed a real bug found on the way: under ble.sh a scraped `claude` pane resolved to `Harness::Absent` and got no conversation at all |
| **W4** | Done | `error.node`, written test-first against a real herdr; `saidOutLoud` decides loudness on the client, where the pane on screen is known. The words are kept on the pane rather than dropped |
| **W5** | Done | `Facets` collected per harness, and `title` wired from marker to sidebar where the naming template renders it. Measured `queue-operation` to have four operations, not two. `convo.facets` is sent when a conversation opens and **again whenever the facets move**: the fold is resumable — it is fed the records the transcript has grown by — and the pump republishes only on a change. Live-tested with a prompt queued mid-turn, which is the shape the operator reported. Both surfaces draw the queue: `QueuedTurn.kt` in the app and `convo::QUEUED` in the terminal, each naming it *queued* rather than as the operator, because the queue is the pane's and a prompt in it may have been typed at the desk. **A prompt the running turn absorbs leaves the queue and becomes a turn of its own** — it is written only as an `attachment` record ([#462](./03-probe-log.md)), so until it was read the operator watched their own message disappear at the moment the agent took it up |
| **W6** | Done, one gap | `Source::Diff`, `git.rs`, `AttachmentIds`, `FileViewer` with "Changes since HEAD". Caught a read-only hub slipping a `diff` id past a gate that matched only `Source::File`. **The terminal surface does not mint ids** |
| **W7** | Done | The `paste` verb, live-tested and mutation-tested; a picker on all three platforms and a handover line |
| **W8** | Done | The procfs walk fills `cmd`/`argv` only where herdr has no answer. Six mutations, all caught |
| **W9** | Done | `theNotificationIconFillsItsKeylineAndIsCentred`, run on a real emulator; both assertions independently load-bearing |
| **W10** | Desktop double-count and the 0 dp reply box fixed; **phone offset changed, not verified** | The double-count went red at 88 dp against 44 and is now 62. The theme change is reasoning, and at `targetSdk 37` those attributes may be no-ops — the three device checks are still the test |

Gate at the end of this pass: `cargo fmt --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, **1088** Rust tests and **1282** client tests green, 71 of the Rust ones driving a real herdr.

**What is honestly not done**, and none of it should be read as done:

- ~~The terminal surface mints no file ids and has no attach control.~~ **Done** — a path on the
  grid is now a second fetchable target beside a URL, arbitrated by the same `filePathOf` the
  conversation uses, and an attach pill on the terminal's own chrome hands a file to the pane an
  agent is actually running in. Two things it deliberately does **not** carry over from the
  conversation's viewer: syntax highlighting, and the `git diff HEAD` companion — the second is
  new node surface by W6's own pricing.
- **Free-text path detection in prose** is deliberately absent on the *conversation* side: only
  paths the node itself derived (a tool card's `summary`, a `Diff` block's `path`) are offered. On
  the **terminal** a path has no such provenance — the grid is bytes — so the token under the
  finger is arbitrated by `filePathOf`, which admits only an absolute or `~/`-rooted path.
- ~~A running subagent's card does not live-tail.~~ **Done** — the node follows the sub the reader
  has open and pushes what it grows by as `convo.turn` carrying the same handle, on the pane's own
  tick. One at a time, and nothing at all for a client that never opens one.
- The **phone's window offset**. Changed on a hypothesis, unverified, and no JVM harness can see a
  `Popup`.
- ~~Codex and agy fill no facets.~~ **Done** — the census ran on 11 real Codex rollouts and 26 agy
  transcripts (#322). Codex fills timings (better than Claude's: `duration_ms` is given outright),
  mode, and position-only compactions; it has no title and no queue. agy fills a compaction
  boundary and nothing else. agy timings were refused by name: a `created_at` delta contains the
  operator's own thinking time, which is not a duration the harness recorded.
- A subagent card's state still settles on its tool result rather than on `toolUseResult.status`,
  because the completion signal is unmeasured. Holding a card `Running` on `async_launched` would
  leave every finished subagent spinning for ever, which is a worse lie than the current one.
- `convo.sub` has wire-level tests but no end-to-end test that a frame reaches `open_sub` — the live
  harness runs a shell, not an agent.
- ~~At `ime = 280 dp` on a rotated pane the reply field measures 0 dp tall (#319).~~ **Fixed** — it
  had two causes and either alone leaves the other standing (#321): the pane header took the room
  first, and the column then served the reply box last. Both reverts were run.

---

## What not to do

- **Do not inline subagent turns into the parent conversation.** Installed phones would render them
  as the parent's own words.
- **Do not put a harness's own field names on the wire.** Three harnesses, one additive protocol,
  for ever. W5 says how.
- **Do not let procfs become a second herd model.** herdr owns which panes exist. This reads one
  machine about one pane herdr has already named.
- **Do not cite a probe number that does not exist.** W0's rows are unnumbered here deliberately;
  number them on append, then update the citations in this file in the same commit.
- **Do not claim a PTY.** Nothing in this brief needs to, and nothing in it may. The one deliberate
  reshape is the `pane.size` op behind its panel
  ([ADR 0012](./adr/0012-one-deliberate-resize-behind-a-panel.md)); reading a pane — watching,
  walking its procfs, tailing its transcript — never reshapes it.
