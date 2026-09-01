# Agent status, transparently

**The goal, in the operator's words:** *transparently track agents, understand their session
files, keep the conversation pane up to date in the least fragile and most immediate way
possible.*

Three constraints fall out of that, and they are not negotiable:

1. **Transparent.** Kampr is pointed at a machine and reads whatever is already running. It may
   not require the operator, or any Kampr user, to install a hook, an integration, an MCP server
   or any other configuration into their agent. A mechanism that only works for sessions Kampr
   launched is not a mechanism.
2. **Least fragile.** Where a harness publishes something about itself, read that. Where it does
   not, say so. **Never guess**, because a plausible-looking wrong answer is worse than an empty
   one — that is the whole lesson of #233 and of the conversation-pane defects below.
3. **Most immediate.** Push over poll where a push exists; a bounded poll otherwise.

This page is the record of what was measured, what was decided, and what is still open. Probe
rows drafted here are **unnumbered on purpose** — the log is append-only and gap-free, another
workstream is appending concurrently, and one writer assigns numbers (rule 1).

---

## 1. Why the conversation pane could not be trusted

Two defects, one root, both now fixed. Kept here because the mechanism explains the status work
that follows.

`Registry::locate` (`crates/kampr-journal/src/adapter.rs`) resolves a pane's transcript through
three handles, strongest first: the announced session, the harness **process**, then the working
directory. Every failure at a strong handle was swallowed with `.ok()` and fell through to the
directory search — and that search is a **time bound, not an identity** (`process.rs`'s
`active_since`). Any sibling transcript in the same project directory still being appended to
passes it.

- **Before the first message.** The marker `~/.claude/sessions/<pid>.json` exists from session
  open, but the transcript is not created until the first submit — **2 min 42 s**, measured
  (#311). In that window `locate_by_process` returned the same `NotFound` for "marker found,
  session known, transcript not written yet" as for "no marker at all", so the ladder guessed,
  and answered with the pane next door (#260).
- **Mid-session.** `/clear` opens a new transcript under a new session id but keeps the **same
  pid and `procStart`** (#259). The start-time bound therefore does not move, the pre-`/clear`
  transcript still passes it, and the pane re-bound to the file it had just left. That file
  never grows again, so the conversation froze while new messages went somewhere unseen.
- **And the freeze had a second path.** `release` deliberately tells the client nothing — its
  own doc says *"what the client is holding is then whatever an earlier one put there, and it is
  still on the screen"* — and it was called on every handle move, every unreadable transcript and
  every disagreeing recheck. Only a *successful* resolve could ever take a conversation off the
  screen.

**Fixed by:** a distinct `JournalError::Unwritten` so the marker's third state stops being folded
into "no marker" (`marker.rs` had already specified this and forbidden the drift); the ladder
stopping on it rather than guessing; and a `retire` on a handle move that names a *different*
session. Narrowed by `moved()`, which fires only when two session names disagree — see §2 for why
that narrowing is load-bearing.

**Tests:** `crates/kampr-journal/tests/identity.rs::a_pane_whose_session_has_written_nothing_shows_nothing_though_a_shell_run_is_newer`
and `crates/kampr-node/tests/live.rs::a_session_that_has_written_nothing_takes_the_previous_conversation_off_the_client`,
both written red first. `moved()`'s narrowing is pinned by
`convo::tests::a_pane_that_stops_naming_its_session_keeps_the_conversation_it_had`, mutation-checked:
defeat `moved()` and it fails.

---

## 2. How herdr really derives agent status, and why it blinks

**Probe #75 describes a third of it.** Detection in 0.8.2 is two stages:

- **The label** (`agent`) is the **foreground process name, with no verification whatsoever.** A
  copy of `/usr/bin/cat` renamed to `claude` reports `agent: "claude"` and is served the real
  Claude manifest. Conversely a perfect Claude permission prompt printed under plain `bash`
  produces no agent at all — screen text never *creates* a label.
- **The state** (`agent_status`) comes from versioned per-agent **manifests**, not from the
  config. Three sources per agent — bundled (20 in the binary), remote (fetched from
  `https://herdr.dev/agent-detection/index.toml`), and a local override — highest version wins.
  The claude manifest carries 16 rules over seven screen regions (`osc_title`, `osc_progress`,
  `bottom_non_empty_lines(N)`, `whole_recent`, `prompt_box_body`, `after_last_horizontal_rule`,
  `last_non_empty_above_prompt_box`), matched **strict highest-priority-wins**.
  `herdr agent explain --json` is the instrument: it returns every rule with its regexes and a
  `region_preview` of the exact bytes scanned.

**herdr does not flap.** It debounces: contrary evidence must persist **between 200 ms and
400 ms** to move the published state. 20 000 lines of burst output produced two transitions. Under
a 20 Hz alternating title the raw verdict oscillated every ~40 ms while the published status
stayed pinned. Latency to the snapshot is **under 0.4 s** (median ~0.2 s).

**So the blinking is not flapping. It is also not a label drop — that reading was measured once,
published here provisionally, and has since been refuted (#360).** What is real is sharper and
worse:

> `pane.terminal_title` and the `osc_title` **detection region are different state**. The region
> records only titles written *after* herdr attached the label — the boundary is 0.15–0.3 s past
> `exec`, herdr's own detection tick — and it is **never backfilled**. A harness that titles
> itself inside that window leaves herdr publishing `default_known_agent_idle_fallback → idle`
> **indefinitely**, at a pane whose title on screen says working. Re-writing the byte-identical
> title fixes it in under half a second.

The refuted half, recorded so nobody re-derives it: a child holding the pane's foreground does
**not** make herdr forget the agent. Measured to 20 s with the agent evicted from the foreground
process group entirely, and over 300 s on a live Claude pane — where a Bash tool child is spawned
into **its own** process group and never becomes the tty's foreground at all, so
`foreground_processes` reads `['claude']` throughout. The three-second figure was real but
belongs elsewhere: herdr holds `unknown` for **3.33 s** after a label attaches with nothing
matching, before the idle fallback publishes (~0.35 s when a rule matches).

**`moved()` is still load-bearing, for a corrected reason.** There are two genuine blinks — that
3.33 s of `unknown`, and a single `idle` herdr emits as an agent exits — and a conversation
withdrawn on either is a pane emptying itself while nothing is wrong.

Two more rules of engagement, measured:

- **Precedence is `matching rule > report > idle fallback`.** `pane.report_agent` never beats a
  live rule, whatever its `source`. Against the fallback it wins.
- **A report never expires** — 400/400 samples over 406 s, `revision` unmoved — and a report on a
  pane with no agent *invents* one, unverified.
- **13 of 17 herdr integrations, including claude and codex, report session identity only, never
  state.** So Claude's lifecycle state is screen-scraped even with herdr's own integration
  installed.

---

## 3. What Claude publishes about itself, with no configuration

### `claude agents --json` — the load-bearing find

First-party, and its own `--help` says *"Print active sessions (interactive and background) as a
JSON array and exit (for scripting; does not require a TTY)"*. **"For scripting" is a blessing the
transcript JSONL explicitly does not have** — the docs say scripts should not parse transcripts.

It lists sessions **it did not start**:

```json
{ "pid": 3116052, "cwd": "/home/dbrain/dev/kampr", "kind": "interactive",
  "startedAt": 1787960796254, "sessionId": "456945f8-…",
  "name": "kampr-44", "status": "busy" }
```

Every join Kampr needs is in there: `pid` joins to the pane through the procfs walk already
built, `sessionId` joins to the transcript, `status` is what herdr screen-scrapes for.

**Not observed:** `state` and `waitingFor` are reported to be documented fields but did **not**
appear in output on 2.1.251 for interactive sessions. Treat as unconfirmed; plan against `status`
until someone catches a session at a permission prompt.

### The marker's `status` — W0 resolves

`docs/12-harness-briefs.md` carried *"Whether `sessions/<pid>.json`'s `status` tracks a live
session or is written once — guess, W0"*, gating whether W3 could **replace** herdr's
`agent_status` or only supplement it.

**Measured: it tracks.** `statusUpdatedAt` drifts **9352 min, 86.5 min and 132.9 min** past
`startedAt` across three live sessions, and a newly created session was seen going
`status: absent → idle` 0.4 s after its marker appeared. It is edge-written, not a heartbeat:
`updatedAt == statusUpdatedAt`, frozen between transitions. **W0 resolves in favour of using it.**

### `lsof` does not work, and it is worth recording why

Claude opens its transcript, appends and closes. Sampled 60 times on a session actively writing:
the `.jsonl` appeared in `/proc/<pid>/fd` **0 times**. The fd table *does* hold an open directory
handle naming the session id (`/tmp/claude-<uid>/<slug>/<sessionId>/tasks`), which is kernel-held
and so cannot go stale — but the marker is already `procStart`-validated against `/proc`, so the
staleness hole the fd would close is closed. The fd path is an unversioned internal scratch
layout; the marker is at least a stable artefact with a probe row. **Prefer the marker.**

### The marker is the push signal — `IN_MODIFY`, not a poll

Measured on 2.1.251 (`GIT_SHA 37534ac5`), and this is the most useful thing on this page:

- **`status` has four values, not two: `busy | shell | idle | waiting`**, and a `waitingFor`
  beside it (`"input needed"`, `"worker request"`, `"sandbox request"`, `"dialog open"`, or the
  top dialog's own label). `shell` is idle with a background shell task running.
- **It is rewritten in place** — `publishDiscipline: "inPlace"`, inode unchanged across every
  flip. So a plain **`IN_MODIFY` inotify watch on `<pid>.json` is a push feed**: no handshake, no
  socket of Kampr's own, nothing printed in the operator's transcript, and it covers **all** the
  states including blocked.
- **Latency ~100 ms**: `idle→busy` 100 ms after a prompt, `idle→waiting` 100 ms after a dialog
  opened, `busy→idle` at the same millisecond the socket's own notice reports.

### `cc-socks` — measured, and *not* what to build on

Newline-delimited JSON over `AF_UNIX`, **one direction**; nothing is ever pushed down the
connection you opened (EOF at exactly 30 s). `peerProtocol: 1` is advertised and **never read**.
On Linux **auth is optional** — the socket's `0700` directory is the real access control, alongside
`SO_PEERCRED`.

`notify_idle` is a genuine one-shot subscription: a peer registers `notify_when_idle` naming its
own socket, and the session **dials out** to it with `peer_idle_notice`. It works with zero
configuration. But three things rule it out as the primary:

1. **It cannot say "blocked".** The feed is wired straight off two booleans —
   `e0t(R==="idle"||R==="shell", R==="busy")` — and `waiting` is neither, so **a session blocked
   on input fires nothing at all.** Measured: 40 s of silence while blocked on a dialog, then a
   notice 750 ms after it closed. That is the one state Kampr most wants.
2. **It is not invisible.** The target prints the subscription into its own transcript, so a
   per-pane watcher re-subscribing every turn writes noise into the operator's scrollback.
3. **It is behind a server-side kill switch** — `tengu_harbor_kite`, default true, remotely
   flippable. When it flips, subscriptions return `refuse` and go **quiet rather than erroring**:
   the #233 shape, on a machine that worked yesterday.

`state: "exited"` does fire promptly (within 10 ms of `SIGTERM`), which is the one thing the
socket does that the marker cannot. Treat the socket as an optional accelerator, never the signal.

`reply_across_default_dirs` is a sender-side widening Kampr does not need (binding beside the
target's own socket already passes). `artifact_yield` is unrelated — it hands the live
artifact-comment watch between two processes of the *same* conversation on `--resume`.

---

## 4. The design that falls out — three layers, no user configuration

| layer | source | for |
|---|---|---|
| 1 | **`IN_MODIFY` on `~/.claude/sessions/<pid>.json`** | status, pushed at ~100 ms, covering `busy \| shell \| idle \| waiting` **and** `waitingFor`. No configuration, no socket, nothing written into the operator's transcript |
| 1b | `claude agents --json` | enumeration and bootstrap — what sessions exist, and the supported cross-check on the marker. The only *documented* surface of the three |
| 2 | the transcript JSONL | conversation **content** — still the only source of the original markdown. Undocumented: pin the parser, expect churn |
| 3 | the grid, via herdr | last resort, and **not deletable** — see below |

**Layer 3 does not get deleted, it gets demoted.** Sculptor built hook-driven state detection and
documented that an **Esc interrupt fires no hook at all**. The screen is the only thing that
catches it. herdr's scrape stops being primary; it does not stop existing.

**Hooks are out**, and for a stronger reason than the configuration constraint: hooks are loaded
at **session start**, and one added mid-session fires only for subsequent events. Even if Kampr
installed them, they would never reach a pane already running — the panes Kampr exists to adopt.
`--settings` injects hooks per-invocation without touching user files, but only for sessions
launched through it. Hooks remain a thing `kampr doctor` may *offer*; never a requirement, and
never installed behind the operator's back.

**ACP and MCP both fail the constraint.** ACP's model is that the client **spawns** the agent and
owns its stdio — no observer role, no third-party subscription, and Anthropic closed native ACP
support as "not planned". MCP is agent→tools; its notifications are scoped to an in-flight tool
call, so it can never report "idle" or "blocked at a prompt", and it must be installed into the
agent's config.

### Cross-agent coverage — measured

Three of the four publish a live status transparently. The ladder is per-harness by design;
`JournalAdapter` is the seam.

| | identity handle | status signal | blocked? |
|---|---|---|---|
| **claude** 2.1.251 | `sessions/<pid>.json`, pid-keyed, `procStart`-validated. From session open | `status` + `waitingFor`, in-place, ~100 ms | **direct** |
| **codex** 0.150.1 | **`thread-writer-locks/<id>.lock` held under `flock` for the process's life**, plus the rollout fd held open continuously. From **process start** | `thread_history_1.sqlite` `thread_turns.status` (`inProgress`→`completed`), or the rollout's `task_started`/`task_complete` | inferred only (unmatched tool call, #43) |
| **agy** 1.1.22 | `presence/<conv-id>.lock` under `flock` — but **only from the first prompt**; `crashes/crash_<pid>_<uuid>.log` covers boot | `conversations/<id>.db` `steps.status`: 8 generating, 9 **awaiting approval**, 3 done, 6 cancelled | **direct, and the only one that is a value rather than an inference** |
| **gemini** 0.56.0 | **nothing** — 0 regular-file fds and 0 lock entries across 643 samples, no env var | none found | no |

**Two corrections to this codebase fell out of that.** `codex/mod.rs` claimed codex *"publishes no
map from a process to the thread it is on"* — the lock files are empty, which is what made them
look like nothing, but they are `flock`-held from before the first prompt to process death.
Kampr is leaving its strongest codex handle on the floor. And `marker.rs` still said Claude's
`status` was unmeasured. Both comments are corrected; **wiring the codex lock is not done** and is
the obvious next piece.

Two traps for whoever wires codex: the lock is held by the **native** binary, not the
`bin/codex.js` wrapper that spawns it, so the pipeline walk must reach the child; and `/new`
takes a **second** lock without releasing the first, so agy's "exactly one held lock or nothing"
rule would refuse a session that has used it. Disambiguate by the open rollout fd.

**gemini is the weakest by a wide margin** and should be labelled as such rather than guessed at:
cwd plus mtime is the same time-bound-not-identity that caused §1. Note also that gemini 0.56.0
**could not authenticate on this machine at all** (*"This client is no longer supported for Gemini
Code Assist for individuals"*), so everything recorded about it is boot-time only.

---

### What the pipeline actually costs — measured ([#361](./03-probe-log.md), [#362](./03-probe-log.md))

The constants in `convo.rs` read far worse than the pump behaves, and the one number that *is* bad
is not any of them.

| | measured |
|---|---|
| `watch` → first page, 46 turns | **2 ms** median |
| `watch` → first page, 4000 turns (9.3 MB) | **117 ms** median — the fold's first read |
| new turn on disk → `convo.turn` on the wire | **209 ms** median, 398 ms worst — `POLL` and nothing else |

Neither `POLL` nor `RESOLVE_EVERY` is on the first-page path: the pump enters its loop with
`due = true`, so resolve, page and drain all run before a tick is awaited. **A warm conversation is
not where latency lives.** These measure the node from file write to wire and exclude how long a
harness takes to flush a record, which is upstream of everything here.

**The cold path is a different story, and it has a defect.** With the watch sent before any
transcript exists, the first page arrives at 2 s / 2 s / 4 s / 6 s for a transcript written at
0.3 s / 1 s / 3 s / 5 s — and at **15 s** for one written at 7 s *or* at 11 s. `RETRY_EVERY` with
`FAST_RETRIES` reads as a 10 s fast window and measures as 6 s, because at t≈0 the initial `due`
resolve and `retry`'s own immediate first tick each burn a miss. So the ladder is 0, 0, 2, 4, 6 and
then silent until `RESOLVE_EVERY`. **A transcript that materialises at 7 s costs 8 s of dead air —
worse than one that materialises at 11 s**, which is the non-monotonicity. `recheck` consumes its
immediate tick before the loop; `retry` does not, and that asymmetry is the bug.

Not yet fixed: the one-line change is `retry.tick().await` beside `recheck`'s, which moves the
quiet point to 8 s and the worst dead air to 5 s. It wants the measurement harness committed with
it — a timing change with no guard is how the next one gets re-tuned by accident.

---

## 5. Decisions taken

- **Sidebar order is herdr's order: `blocked > done > working > idle > unknown`.** `done` is only
  ever synthesised for a pane that went `working`→`idle` while **unfocused** — an unread marker,
  so it is news where `working` is not. Landed in both surfaces (`sidebar.rs::rank` and
  `Herd.kt::statusRank`) with a test each, because two clients that order one herd differently are
  two different products — the defect `9a52a3e` fixed.
- **Focus is now named in rule 3.** It is not a resize and not a read, but it is the **only** thing
  that destroys herdr's `done` marker; every read leaves it standing. Not `pane.focus` alone:
  `tab.focus` and `workspace.focus` destroy it exactly as `pane.focus` does, because what herdr
  answers to is *the pane becoming the session-focused pane*, however it got there — and Kampr's
  `focus` manage op routes all three (`manage.rs`). Focus is a thing the operator presses, never a
  side effect of opening a view. Kampr already never focuses implicitly — every create op passes
  `focus: false` — so this writes down a discipline that already held.
- **A marker saying `idle` may not demote `done`.** The harness outranking the screen is right for
  every word but this one: a finished Claude session writes `status: "idle"` at the same moment
  herdr synthesises `done` for the same pane, and `done` is that `idle` plus the half the marker
  never knew — nobody has looked yet. `state.rs::settled_status` keeps `done` under an `idle` or
  `shell` marker and lets every other word through, `busy` included: a pane that has started again
  is not one waiting to be read. The symptom was a finished agent rendering grey with the status
  mark suppressed — *"when an agent is done done … doesn't show anything, just goes grey"*.
- **A pane is called what its transcript calls it.** The herd path now carries the session's real
  title — `custom-title.json` before `ai-title` before `agent-name` — through an incremental fold
  cached per transcript path and pruned per round like `Conversations`. Steady-state cost is
  **1.9 us per agent pane per rebuild, flat from 2 KB to 29 MB**, beside the 34 us the marker on
  the same pane already costs; the whole-file read happens once per transcript per node lifetime
  (1.0 ms at this machine's median, 26 ms at its largest). That is the cost `wire.rs` refused to
  pay *per rebuild*, and the cursor is what makes paying it once enough. No wire change: the
  existing optional `PaneEntry.title` is simply filled better, so installed phones are unaffected.
  This also removes the cost accepted below.
- **Harness-derived session names are not shown.** `chosen_name` in `state.rs` drops a name whose
  `nameSource` is `auto`, `derived` or absent; all three are machines naming themselves. *Known
  cost:* two Claude panes in one workspace now render identically, where `kampr-44` vs `kampr-1f`
  at least distinguished them. The good names (`ai-title`, `agent-name`) exist in the transcript
  but cost a whole-transcript read per pane, which the herd path deliberately avoids. **Open.**
- **Codex's writer lock is now the codex handle.** `presence` moved out of the `agy` module to
  the crate root, `newest_holder` was added for codex's `/new` behaviour (#350), and
  `CodexAdapter::locate_by_process` resolves the thread through `/proc/locks` and falls back to
  the process's children, because the holder is the native binary rather than the node wrapper
  that spawns it (#349). Two tests take a real `flock` against the real kernel.
- **A harness's own status now outranks herdr's screen scrape.** `harness_status` in `state.rs`
  maps `busy \| shell \| idle \| waiting` onto the herd's five and leaves a word it does not know
  alone. It costs **nothing**: the marker was already being read once per pane per rebuild for the
  title. `waiting`→`Blocked` is the state herdr structurally cannot see (#355), and the live test
  asserts it against herdr saying `working`, so it fails if the override is removed.
- **`state_change_seq` is noted, not wired.** It is a free global monotonic change counter on
  `agent.list`. Wiring it without a consumer would be dead code; it wants a purpose first.

---

## 6. Probe rows — appended as **#343–#362**

No longer pending: the concurrent workstream landed, the log was clean, and these went in at the
end of it. #343–#348 are the Claude surfaces, #349–#351 codex, #352–#353 agy, #354 gemini, and
#355–#359 herdr's detection model, its report precedence and its session lifecycle; #360 is the
refutation in §2, and **#361–#362** the pump's measured latency and the cold-start cliff. The tables
below are kept only as the short form; the log is the record.

| # | Claim | How | Result |
|---|---|---|---|
| | **`claude agents --json` lists interactive sessions the caller did not start, with pid, session id and live status, and needs no TTY and no configuration** | `claude agents --json` and `claude agents --help` on 2.1.251, against three sessions started by hand | Returns a JSON array of `{pid, cwd, kind, startedAt, sessionId, name, status}`; the three live sessions read `busy`, `idle`, `idle` matching reality. `--help`: *"Print active sessions (interactive and background) as a JSON array and exit (for scripting; does not require a TTY)"* — an explicit third-party contract the transcript JSONL does not have. **Not observed:** the `state` and `waitingFor` fields reported as documented did not appear for interactive sessions |
| | **`~/.claude/sessions/<pid>.json`'s `status` tracks the live session — it is not stamped once** | compared `statusUpdatedAt` against `startedAt` on three live sessions, and watched a fourth from creation at 0.4 s sampling | Drift of **9352 min, 86.5 min and 132.9 min** past `startedAt`. A newly created session's marker appeared with **no `status` field** and was rewritten `idle` 0.4 s later. `updatedAt == statusUpdatedAt` and both freeze between transitions, so it is edge-written rather than a heartbeat. **Resolves W0** in `docs/12-harness-briefs.md`: the marker's `status` may replace herdr's screen-scraped `agent_status`, not merely supplement it |
| | **Claude never holds its transcript open, so no `lsof`-shaped handle finds it — but the fd table does name the session** | sampled `/proc/<pid>/fd` 60 times on a session that was actively writing its transcript | The `.jsonl` appeared **0 of 60 times**: it opens, appends and closes. The fd table does hold an open directory handle `/tmp/claude-<uid>/<slug>/<sessionId>/tasks`, kernel-held and so incapable of going stale — but the marker is already `procStart`-validated, so it closes no hole the marker leaves, and it is an unversioned scratch layout. Prefer the marker |
| | **A live Claude session listens on `/run/user/<uid>/cc-socks/<pid>.sock` and advertises `notify_idle`** | read `messagingSocketPath`, `peerProtocol` and `peerFeatures` off live markers; `ls -la /run/user/1000/cc-socks/` | One `srw-------` socket per live session pid. `peerProtocol: 1`; `peerFeatures: ["notify_idle", "reply_across_default_dirs", "artifact_yield"]`. Undocumented. **Protocol not yet measured** — nothing may be built on this until it is |

Further rows on herdr's detection model — the two-stage label, the manifest system, the debounce
window, the `done` semantics, the report precedence and the label-drop defect — are drafted in the
agent-status probe report and belong beside these.

---

## 7. Open, and being probed

- **The `cc-socks` protocol.** Framing, handshake, and whether an external process can subscribe
  to `notify_idle` with no configuration. If yes, layer 1 becomes push rather than poll.
- **agy's status is measured (#353) and deliberately not wired.** Three reasons, and they are the
  "stable rather than clever" ones: the `steps.status` enum is **only partly mapped** — four
  values observed out of an unknown number — it would mean opening another process's live SQLite
  once per pane per rebuild where every other signal here is a file read, and the adapter surface
  it would hang off is synchronous. Identity for agy already works through its presence lock.
  Worth doing once the enum is fully mapped; not worth guessing at.
- **gemini has no handle at all (#354)**, so it stays on herdr's label and the bounded directory
  search, and should be *labelled* as the weakest rather than quietly treated as equal.
- ~~**Whether to fork or merge with herdr.**~~ **Answered: do neither.** Herdr is forkable —
  `github.com/herdrdev/herdr`, Rust, and **relicensed from AGPL-3.0 to Apache-2.0 on 22 Jul
  2026**, which is what made it possible at all (AGPL §13 would have forced this MIT node to
  relicense or buy a licence). But it takes no unsolicited PRs, moves at ~8.6 commits/day
  concentrated in exactly the files a fork would patch, and a fork breaks the premise rule 3
  rests on: the operator would be running *Kampr's herdr*, not herdr.
  ~3 650 lines — **4.4%** of Kampr's Rust — are pure workaround, and the largest chunk exists for
  a trivial reason: **`src/pane.rs`'s `current_size()` already returns `(u16, u16)` and the
  `PaneInfo` builder already holds that runtime**, so §4.2's 767 lines of width inference exist
  because one `u16` is missing from a struct that has it in scope. That is an upstream ask, not a
  fork. Eleven such asks (U1–U9, U8b, U8c) are **already written and still unfiled** in
  `docs/02-roadmap.md`; `ARCHITECTURE.md` §9 already says to file them.
  Related correction: the "undocumented herdr API" worry is largely wrong — `terminal session
  observe` and `control` **are** documented, just hidden from top-level `--help`.
- ~~**`waitingFor`** — documented, never observed.~~ **Observed**, on a live session sitting at a
  dialog: `{"status":"waiting","waitingFor":"dialog open"}`. A second value beside
  `"permission prompt"`. Whether `claude agents --json` surfaces it as well as the marker does is
  still untested — the observation is marker-only.
- **A same-uid process can inject a real user turn into any live Claude Code session with no
  authentication**, by writing `{"type":"user",…}` to that session's `cc-socks` socket. The
  directory being `0700` is the entire access control. This is a fact about any machine a node
  runs on, not about Kampr — but it belongs in `docs/08-threat-model.md`, and it is also a
  non-PTY path to the agent if `convo.composer` ever wants one.
- **Socket does not imply marker.** A Claude process spawned as a child of another
  (`CLAUDE_CODE_CHILD_SESSION`) gets a socket and a key file but **no `<pid>.json`**. Anything
  walking the registry must not assume the two go together.
### Background sessions have no pane — designed, deliberately not landed

`claude --bg` sessions are separate Claude Code sessions with their own session ids, supervised by
a daemon, with **no terminal pane at all**. Herdr will never see one; `claude agents --json` (#343)
lists them and marks them with `kind`. The operator wants them visible.

**Half of it is already precedented and easy.** `Provider` is a trait and `sessions.rs` already
composes two implementations behind a `Composite` — the fleet workstream added `FleetProvider`
beside the herdr one for exactly this reason, panes the node owns rather than panes herdr has. A
third provider listing background sessions as entries under a `bg:<session-id>` local id follows
that pattern, answers `owns` on the prefix, and returns nothing readable for `watch_pane` and
`read_scrollback` — which is an existing, rendered state, not a new one.

**The other half is the actual work, and it is why this is not landed here.** A background session
is only worth showing for its *conversation*, and the conversation pump is herdr-shaped: `pane_of`
builds its `Handle` from the herd model's `agent`/`cwd` plus a `Look` that resolves identity
through `HerdrProvider`'s process pipeline. A session with no pane has no pipeline — its identity
is the session id, which is already exact and needs none of that machinery. So making it work
means giving the pump a second way to be told what conversation a pane is on, rather than always
deriving one. That is a change to the most defect-prone code in this crate, landed at the same
time as a new provider and a client that has never drawn a pane with no grid.

Doing all three at once is how the defects in §1 got written. The order that keeps it honest:
generalise the pump's identity source first, with tests, on the panes that already exist; then add
the provider; then the client. Nothing here is blocked — it is sequenced.
