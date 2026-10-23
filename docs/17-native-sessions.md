# Native sessions

**Status: phases 1–6 built** (`crates/kampr-keep`, `crates/kampr-native`, the `native.*`
manage ops, `kampr new`/`ls`/`attach`, the doctor `keep` line, the client's "Kampr session", fleet
runs on the keeper, Claude's status over `OSC 7501`, the per-connection reflowed view). Panes Kampr owns from the PTY up, served beside herdr's through the
same `Provider` seam, and optionally *hosted* in a herdr pane so they show up at the desk too.

Every row cited here exists. The compat layer's questions were measured in #575–#584, what Claude
Code gives a PTY owner in #587–#588, and how the keeper survives upgrades and crashes in
#589–#590. §"What the probes found" has the answers and the changes they made to the design.

## Why

[`ARCHITECTURE.md`](../ARCHITECTURE.md) §4 is mostly a list of things herdr cannot tell a bridge.
Each one has a fix that needs nothing except owning the PTY:

| herdr fact | what it costs today | native |
|---|---|---|
| no API reports the PTY width (#221) | ADR 0014, the bounded `selection.read` | we called `TIOCSWINSZ`, so we know it |
| frames are coalesced grid state, 29 lines out of 200 (#25) | ADR 0004: stitching, gap discard, no deep history (#51) | the emulator is fed every byte, so scrollback is complete — for programs that have one. Claude's fullscreen renderer has none (#587) |
| content travels on a spawned child, structure on a socket (#233) | a node that looks healthy while every pane is blank | one connection, one failure mode |
| no output event (#4, #5), no resize event (#52) | a 3 s / 30 s sweep | the log grows, and inotify says so |
| `agent_status_changed` per pane, one bad entry kills the call (#54) | re-subscription bookkeeping | not applicable |
| a deep `text` read can inject wheel events for 5.3 s (#513) | the format interlock | not applicable |
| `/proc` is EPERM to a non-ancestor (#331, #332) | fleet runs had to become their own provider | the keeper is the ancestor |
| `done` is armed and destroyed by herdr's focus rules (#357, #396, #399) | `settled_status` (#395) | we define `done` |

What does **not** change: structure still comes from the transcript (ADR 0005), the wire is still a
cell grid (ADR 0003) so no phone needs a release, and a full-screen program still draws for exactly
one size. A PTY has one geometry and owning it does not change that. What changes is that the
policy is ours and every input to it is an event rather than an inference.

## The shape

```
 phone / browser / kampr tui          herdr desk
          │                              │  (a herdr pane running `kampr attach s7`)
          ▼                              ▼
     kampr serve ──────────────┐   kampr attach
     NativeProvider            │         │
     kampr-term per session    │         │
          │                    ▼         ▼
          └──── log + fd ──► kampr-keep@<id> (one per session, systemd user unit)
                          │  the PTY master, the child, its log on tmpfs
                          ▼
                    claude / codex / bash …
```

Three new pieces: a **keeper** per session that owns its PTY, a **provider** in the node that reads it, and an
**attach client** that puts a session in any terminal — a herdr pane being the case that matters.

### 1. `kampr-keep`: one keeper per session, and nothing else that has to survive

The property that makes native sessions worth having is that **nothing but the program's own exit
ends it**: not a node upgrade, not a node crash, not an upgrade of the piece that owns the PTY, and
not that piece crashing either. #589 and #590 measured how to get all four, and the answer is to
make the part that must survive so small that everything else can restart around it.

A keeper is one process per session, a `kampr-keep@<id>.service` systemd user unit, and it does
five things:

- forks the program on a PTY it allocates, setting `KAMPR_SESSION_ID`, and keeps the master;
- drains the master continuously (a full PTY blocks the child; the herdr client in #507 had the
  same need) into an **append-only log on tmpfs**, `$XDG_RUNTIME_DIR/kampr/s/<id>/log.<offset>`,
  segmented so the oldest segment can be deleted and every byte keeps its absolute offset;
- hands the master to systemd's fd store once, at spawn (`FDSTORE=1`, `FileDescriptorStoreMax=1`);
- on `SIGUSR1`, `execve`s whatever binary is now at its own path with the master left open;
- writes the program's exit status to `exit` in the same directory, and exits.

It holds no emulator, serves no clients and speaks no protocol beyond one-line requests on
`ctl.sock`. **Everything else reads the directory and borrows the master**: the log is a file the
node or `kampr attach` tails from any offset, and a client that needs to type gets a dup of the
master over the keeper's `SCM_RIGHTS` socket (`fd`) and calls `write` itself. **Nobody but the
keeper reads the master, and nobody but the keeper resizes it.** A size is asked for with
`resize <cols> <rows> <holder>`: the keeper drains whatever the master holds into the log, appends
`{seq, offset, cols, rows, holder}` to `sizes.jsonl` at the log's end, and only then calls
`TIOCSWINSZ` and rewrites `size.json`. Every byte the program wrote before the resize therefore
precedes its record (#599), and a replay that applies each record at its offset
(`kampr_keep::SizedReader`) rebuilds the screen at every size it had rather than feeding the whole
log at the last one. The spec's size is the size at offset 0, and the journal is trimmed with the
log, keeping the latest record before the oldest retained byte: the size a replay starts at.
Signals are a `kill` from the same uid. So the node, the attach client and all the protocol around
them restart whenever they like, and replaying a log rebuilds their emulators exactly. Every reader
of the log goes through `SizedReader` (the node's tail, `kampr attach`, the fleet ladder); the
plain byte reader under it is no longer exported.

A log is capped (`spec.log_cap`, 32 MiB by default), and a streamed Claude reply costs about 13×
its text (#587), so a long session does trim, and a replay from the oldest retained byte would start
mid-stream: inside a sequence, with the wrong pen, modes and screen. So **the node** (never the
keeper, which stays tiny) writes a **checkpoint** into the session directory,
`screen.<offset:020>`: the whole emulator state at that log offset, every 512 KiB of output and
when the node shuts down cleanly, written to a `.tmp` and renamed, the newest two kept. A replay
starts from the newest checkpoint whose offset the log still holds and reads on from there; `kampr
attach` uses the same loader. With none usable it starts at the oldest byte and logs that it is
replaying mid-stream. A checkpoint is taken only where the parser is in ground with no half a
character held, and only when the journal agrees with the screen's size at its offset; one the
log was trimmed past while no node ran is never used, since splicing it to the oldest byte would be
wrong rather than merely incomplete, and one that does not decode, carries another offset than
its name or disagrees with the journal's size is passed over.

What a checkpoint holds: both grids (the primary with alacritty's history window, the alternate
when it is up) cell for cell, each grid's cursor and saved cursor with the pen and the G0–G3
designations, the history ring Kampr renders, the title, every mode (mouse, bracketed paste,
focus, origin, insert, LNM, keypad, cursor keys), the kitty flags on top of each screen's stack,
the scroll region, the tab stops, which of G0/G1 is invoked, the colours a program set (`OSC
4`/`10`/`11`/`12`), the cursor style and the last `OSC 7501` report. alacritty keeps the region,
the tab stops and the invoked charset private, so they are read by probing the live terminal (the
region through origin-mode addressing, the stops through `CHT`, the charset through one character
drawn and put back) and set again on load by the sequences that set them. A restored screen is
proved identical to the one it came from, and to one never checkpointed, through more output,
resizes both ways and sequences that use each piece of private state
(`crates/kampr-native/tests/it/checkpoint.rs`). Not carried: the kitty and title stacks below
their tops, and the alternate screen's saved cursor from an earlier visit to it. For Claude, a
checkpoint at the full 10 000-row history is 106 KB and takes 8 ms to write and 8 ms to load
(release build); under the fullscreen renderer, with no history, 1.8 KB and 0.3 ms.

What each kind of restart costs, measured:

| event | what happens | cost |
|---|---|---|
| node restart, crash or upgrade | the node re-reads the directory and replays each log from its newest checkpoint, each resize at its byte | none (the log is not the node's); a log trimmed past every checkpoint replays from mid-stream |
| keeper upgrade | `SIGUSR1` after the binary is replaced, which the node sends by itself: at its start (so after every `kampr update`) and every minute, to each keeper no longer running the file at its own path: each keeper records in `keeper.json` the file it started from (`image`, a device and inode), which a Mac can tell as well as Linux, and one from before the record is read from `/proc/<pid>/exe` (#598). Same pid, same parent, no SIGHUP, no gap in 1.2 M lines (#589) | none, if the new keeper accepts the old one's handover: the master's fd number, the child pid, the log offset. Keep that format frozen |
| keeper crash | `Restart=always` brings it back with the master from `LISTEN_FDS`; the child blocks on the full PTY meanwhile (#590) | at most the one read in flight (13 bytes in three of five runs). The child's parent is now the user manager, so its **exit status is lost** and `exit` records only that it ended. `state` (below) is also EPERM from then on |
| without the fd store | the child gets SIGHUP and dies (#590's control) | the session |

`KillMode=process` is required, or systemd kills the child with the keeper. `session.close` signals
the child's process group, and the keeper exits when the master reaches EOF.

**One thing does want the keeper's ancestry.** Under yama, `/proc/<pid>/{syscall,wchan}` is EPERM to
anyone but an ancestor (#331), and `kampr-fleet/src/waiting.rs` needs it. So the keeper answers a
second request, `state`, with those reads for its descendants. It is the only logic in the keeper
that is not plumbing, and it should stay that one function. **Not established**: whether
`prctl(PR_SET_PTRACER)` set in the child before `exec` would let the node read those files itself,
which would take `state` out of the keeper too.

`list` is the directory listing. `events` is inotify on it. A desk pane showing the session is a
`hosts` file the node writes (§4). None of these needs the keeper.

### 2. `NativeProvider` (`crates/kampr-native`)

An implementation of `kampr_core::provider::Provider`, registered in `Composite` before
`HerdrProvider`, discriminating on a `n:` pane-id prefix.

| `Provider` method | native implementation |
|---|---|
| `list_panes` | the session directories, `PaneInfo` built with the true size |
| `watch_pane` | the log tailed through `SizedReader` into one emulator per session, shared by every watcher, each resize applied at the byte it happened. A node restart resumes from the newest retained checkpoint (§1) and replays the rest, so screen and history rebuild exactly; a tail that falls behind the trimming resumes the same way, with a `Reset` |
| `write_pane` | a `write` on the borrowed master: bytes as sent, and named keys encoded for the kitty keyboard flags the program pushed (`keys::encode_for`) |
| `read_scrollback` | the emulator's own ring, complete and styled, no interlock |
| `read_rows` / `find` | over that ring. `find` finally runs over styled rows Kampr holds rather than asking herdr (§8 of `ARCHITECTURE.md`) |
| `harness_owns_the_screen` | the emulator knows when the alternate screen is up; no inference |
| `topology` | inotify on the session directories |

**The emulator is `alacritty_terminal` inside `kampr-native`**, not `kampr-term`: herdr did the
emulation first and handed over finished screens, and `kampr-term` only parses that dialect. Each
session's screen is re-serialised as herdr-dialect frames (absolute addressing, SGR, `K`, `2J`,
`?25`, OSC 8), so the registry and every client are unchanged. What that gives over herdr's
dialect and over the `vt100` it replaced:

- rows that scroll off the top go into a ring of rendered rows, numbered from the first that ever
  entered it, each keeping its soft-wrap bit. `Screen::row_wrapped` and `Screen::logical_lines`
  read history and the primary screen as the lines the program wrote, for §3;
- the primary screen reflows on `resize` the way every modern terminal does; the alternate screen
  does not — the program redraws it. Rows already in history keep the width they were written at;
- strikethrough, concealed text, every underline style (as underline: `kampr-term` holds one bit),
  the underline's colour (SGR 58, as `ul` on the wire) and OSC 8 links reach the clients. herdr's
  live stream drops that colour, so a herdr pane shows it only in rows read back through
  `pane.read` (#614). Blink does not: alacritty parses SGR 5 and keeps nothing of it, and carrying
  it would mean forwarding vte's whole `Handler` to `Term`. No client draws blink for either kind,
  so the two render alike;
- a two-column emoji made by a variation selector (`❤️`) is one column to alacritty and two to
  `kampr-term`, herdr and the program; it is drawn wide when the column after it is blank, and as
  U+FFFD otherwise, as every other glyph the two disagree on.

**`FleetProvider` folds into this.** A fleet run is a native session with a `cohort` in its spec
and no host, under `<root>/fleet/<herdr session>/` so neither `kampr ls` nor this provider lists it.
It gets survival across a node restart for free. `kampr-fleet` keeps the fan-out parts (`env.rs`,
`job.rs`, `secretish.rs`) and the waiting ladder (`ladder.rs`, `waiting.rs`, `tail.rs`), which now
reads the keeper's `state` and the log instead of a pty of its own; watching, input, history and
search are this provider's, under the unchanged `fleet:<ULID>` ids. The keeper also ends a session
at most three seconds after its program is reaped, so a leftover child holding the slave cannot
keep a run (or any session) open.

**Agents.** The node spawns the harness, so it knows the pid without walking `/proc/*/environ`
for `HERDR_PANE_ID` (#395). The Claude marker, `claude agents --json` (#343) and the journal
adapters all key off that pid directly, for identity, titles and the conversation.

**Status comes from the harness itself, over its terminal.** Claude Code 2.1.308 asks
`OSC 7501;?` at boot, and a terminal that answers is told the session's state on that channel for
the rest of its life, in the same write as the screen change (#600): `idle`, `working`,
`blocked` with a `kind` (`permission`, `question`, `auth`) and the decoded message
(`approve Bash: touch …`), `done`, `error` with its message, and `clear` when it exits. The marker
carries `waitingFor` too but lags the screen by 3–20 ms and cannot tell done, idle and an error
apart (#610). No hooks, no statusline script, nothing in the operator's settings. Answering changes
no drawn cell; it does drop the `✳`/`◐` glyph from Claude's window title (#601).

- `kampr-native` answers the query in stream order with the rest (`answer.rs`), so ahead of its DA1
  answer: Claude takes a 7501 answer behind DA1's for none (#600).
- Every `Screen` parses the reports, live and replayed alike, so a node restart rebuilds the last
  state from the log. A replayed query is never answered. Reports with an `id` are subagents' and
  are ignored.
- `NativeProvider` maps them onto the herd's five: `working`, `blocked` and `idle` as named, `done`
  as `done` until somebody types into the session from a node (as the output window's `done` is),
  and `error` as **`idle`**: the wire has no error state, and the message is not sent. `clear`
  takes the channel down. `PaneInfo.native.program` carries the kind and message; the dialog
  itself is still read off the screen by `pending.rs`, which needs the options only the screen has.
- While the channel is up, nothing overrides it: not the screen's dialog detector, not the 1.5 s
  output window (`live.rs` `WORKING`), and not the node's marker overlay (`state.rs`). A session
  that never negotiated (any other harness, a shell, a Claude before 2.1.308) keeps all three.
- A hosted session's desk report is built from the same status, so the desk sees it too; herdr
  arms its own `done` from the `working`→`idle` it is reported (#578).

### 3. Geometry

The PTY size belongs to whoever **last explicitly claimed it**, and looking is never a claim. That
is rule 3, restated for a pane we own.

- A **desk client** (`kampr attach`, §4) claims by attaching and by resizing. That is exactly how a
  herdr client already behaves (#507), so the desk keeps the semantics it has today.
- A **phone** never claims by watching. It claims through the existing `pane.size` op and panel
  (ADR 0012), and ADR 0013's standing intent carries over to desk-sized views unchanged.
- With no claimant, a session keeps the size it was spawned at.

There is one thing herdr cannot do and we can: **a per-connection reflowed view of the primary
screen**. The node holds logical lines (the wrap bit above), so a phone watching a shell can be
sent the scrollback and the live primary screen re-laid at *its own* width without touching the
PTY. That is upstream ask U2/U3, built locally, for every pane that is not a full-screen program, and
not for Claude in either renderer, which wraps its own lines (#588).
It is an additive wire flag on `watch`, and a client that does not send it gets today's grid.

**Built** (`crates/kampr-native/src/reflow.rs`). The view is a key of its own to the registry
(`n:<id>~<cols>`), so watchers of one width share one stream and one ring and nobody else's stream
changes. It rides the session's own: each frame the session publishes is a cue to re-lay, and cues
queued behind one are taken with it. History is laid a logical line at a time as it settles and
numbered from the first row ever laid; the screen is laid from the line straddling history onward
up to the last non-blank row or the cursor, and the view's screen is the bottom of that, its top
only ever moving down, as a terminal's does. The cursor maps through the re-wrap by its offset in
its logical line. The node follows the view only while it can be served: with the alternate
screen up, or a harness in the foreground (re-checked every second and on every frame), the
watcher is moved to the pane's own grid and told why (`docs/04`). Through a hub the view is the
peer's: the hub asks for it as a pane of its own on the mesh link (`<pane>~<cols>`), so the hub's
watchers of one width share one stream across the WAN, and a peer too old to serve one is labelled
`unsupported`.

Measured on a 120x40 screen re-laid at 40 with a coloured `cargo`-like log, five 100-column lines a
feed, release build: the emulator and its diff cost 166 µs a feed, the re-lay 274 µs a frame
(184 µs of it rendering the screen's rows, the rest settling history and laying lines, which are
cached by content so a scrolling screen re-lays only its new lines). The view sent 2.41 MB for
1.00 MB of output, against 2.23 MB for the pane's own stream over the same frames.

### 4. The herdr compat layer: a session hosted in a herdr pane

The desk keeps using herdr. A native session appears there as an ordinary herdr pane whose program
is a client of the keeper, the same way `tmux attach` would run inside herdr. We do not fork herdr,
we do not impersonate its socket, and we never touch `herdr-client.sock`.

**`kampr attach <id>`** is a terminal client of the keeper, usable in *any* terminal, not only
herdr:

1. raw mode on its own tty; reports its size as a claim (§3);
2. the newest checkpoint and the log after it, replayed into its own emulator with the node's
   loader (§1), to paint the current screen;
3. then, **while its tty size equals the PTY size, it passes the live bytes through verbatim**.
   herdr sees exactly what the program wrote, so herdr's own scrollback, copy mode and search
   behave as if herdr had spawned the program itself. When the sizes differ (a phone holds a size)
   it paints from its emulator, cropped, with a one-line banner saying who holds the size. The
   program's questions then never reach its terminal, so while cropped, and while it is still
   catching up before its first paint, the attach answers every query past its offset from its own
   emulator, as the node would; passed through, only `OSC 7501` is its own (step 4);
4. keystrokes are written to its borrowed master; its tty's `SIGWINCH` becomes a `resize` request to
   the keeper (§1). It also answers `OSC 7501;?` itself, written to the master as soon as it reads
   the query and **before** it forwards the bytes that asked, because herdr answers DA1 and not
   7501 (#611) and Claude ignores a 7501 answer that arrives after DA1's (#600). A Claude that boots
   inside a herdr pane gets its status channel that way (#612);
5. it reads `HERDR_PANE_ID` from its environment (#507 lists the six `HERDR_*` variables a pane
   carries) and adds it to the session's `hosts` file, so the node knows which desk pane is showing
   the session.

**One answerer per question.** `register_host` records in the host's file the log offset it
registered at (`answers_from`), under a `flock` on the session's `hosts.lock`. A query that ends
past a live host's offset is its terminal's (and its 7501 is that attach's); one that ends at or
before every host's offset is the node's. The node decides under the same lock, after it has read
the query out of the log, so a host registering a moment later has an offset past it and cannot
take it away. Of several attaches, the live host with the lowest pid answers 7501, and only its
terminal is shown the other questions: every other attach passing the bytes through cuts each query
out of what it writes (a query split across two reads of the log is left in). A host file with
no `answers_from` comes from an older attach and is taken to answer everything, so an upgraded node
never answers over it.

**Making herdr believe it.** herdr labels a pane by its foreground process name (#355), which here is
`kampr`, and it has no manifest for that. So the node reports the truth over the socket instead:

- `pane.report_agent` with the harness and Kampr's own status. A report beats herdr's idle fallback
  and never expires (#358), and on an unrecognised pane there is no rule to outrank it. On a
  `kampr` pane it creates the agent label, the `agent.list` entry and the sidebar row (#576);
- `pane.report_metadata {source:"kampr", title}`, which draws on the desk border and survives the
  program's own title changes (#294, #295);
- tokens for the sidebar (#294, #296).

The desk's agents sidebar, its border and its `blocked`/`working` marks then show the native
session as an agent, with Kampr's status, which is better than herdr's scrape anyway (#358: herdr
scrapes even Claude).

**Kampr must not show it twice.** `HerdrProvider` will list the host pane too, with a foreground
process of `kampr`. The registry uses the session's `hosts` file to fold a hosting herdr pane into
its session: one entry on the phone, carrying an additive `on_desk: "w1:p3"` fact. Ops that are
about the desk (`focus`, layout) route to the herdr pane and keep every ADR 0015 restriction; ops
that are about the program (input, size, scrollback, close) route to the session.

**Lifecycle, and the second thing herdr cannot give us.** Closing the herdr pane kills
`kampr attach`, not the agent: the session becomes *detached*, as it would in tmux, and stays on the
phone. Restarting or upgrading herdr detaches every hosted session and kills none of them. A
`native.desk` op splits a herdr pane and runs `kampr attach` in it again. A session started
from the phone can live its whole life with no host at all, as fleet runs already do.

The host's + in the herd starts one of these, on the desk wherever the machine has one: in a
workspace of its own, since splitting the desk's focused pane would reshape somebody's layout. A
session closed on purpose, or whose program exits 0, is forgotten once it ends; a failure stays,
with its exit status, until it is forgotten.

### 5. Wire and CLI, all additive

- `PaneInfo` gains `provider: "native"`, `on_desk`, `detached`, `size_holder`.
- New manage ops: `native.create {args | command, cwd?, label?, desk?, at?}` (with `desk`, it is also hosted
  beside `at`), `native.desk`, `native.close`, `native.forget` — `session.*` was already herdr's
  named sessions.
- `kampr new [--desk] -- <cmd>`, `kampr attach <id>`, `kampr ls`.
- `kampr doctor` gains a `keep` line: sessions kept, units healthy, fd store populated, and any
  keeper still running a binary that has since been replaced (a `USR1` the node has not yet sent).

`tests/it/wire_shapes.rs` covers the additions as it does today.

## What the probes found

Rows #575–#590, reproducible with `research/probe/native-host.py` against herdr 0.9.1 and 0.9.5,
`claude-render.py` against Claude Code 2.1.306 and `keeper.py` under systemd 262. Every
question this section used to ask has an answer, and the compat layer survives all of them.

1. **No op creates a pane running a command** (#575), and `env.SHELL` is ignored. The node splits
   and then types ` unset HISTFILE; exec kampr attach <id>` into the new shell. It lands in under a
   second, typeahead included, so there is no readiness to wait for. Without the `unset`, the line
   goes into `~/.bash_history` on this machine even with a leading space. The line stays visible
   until the attach repaints, so `attach` clears the screen before it paints. This is a write to a
   pane Kampr has just created for itself, which ADR 0015 already permits.
2. **A report is all it takes** (#576). A `kampr` pane is invisible to herdr's agent model until
   then. After it, the pane has the label, the `agent.list` entry, the desk's sidebar row and the
   border title, and the status never expires. `report_agent_session` is accepted and can never be
   read back, so the transcript identity stays the keeper's.
3. **Passthrough is exact** (#577). herdr's visible, ring and unwrapped reads, the alternate
   screen and every resize are byte-identical to the program run directly, so a hosted session
   gets herdr's own scrollback, copy mode and search unchanged. Only an attached client causes a
   resize; a headless split never does.
4. **`done` works better than on a real harness pane** (#578). Reports arm it at every gap, with no
   detection hold to step around (#405), and focus still clears it.
5. **herdr takes its pane processes down in every exit, SIGKILL included** (#579). Only a graceful
   stop restores the layout, and then as fresh shells. With the program owned by a holder outside
   herdr, it survived all three exits; each re-attach rebuilt herdr's scrollback from the replay,
   and two herdr panes on one session mirrored each other (#580). Two consequences for the design:
   - on herdr 0.9.2 and later, re-hosting is herdr's job. A `pane.report_agent` carrying
     `resume_argv: ["env", "KAMPR_KEEP_ROOT=<root>", "<abs kampr>", "attach", <id>]` makes herdr
     run the attach into the restored pane by itself (a leading path is refused along with the whole
     report, #591, and the restored pane does not get the split's `env`, #592), after a graceful stop or a SIGKILL alike, and the log's replay rebuilds herdr's
     scrollback (#583). The report is not restored with it, so the node reports again when the pane
     reappears. On 0.9.1 the field is accepted and ignored: a restored host pane is a bare shell the
     node may type the attach into again, and a session with no saved layout to restore into
     (#582) is *detached* until `native.desk`;
   - `completion_seq` (0.9.2, #584) says a turn finished even after focus has cleared `done`, so the
     desk's unread marker and Kampr's are no longer the same fragile bit;
   - `kampr attach` must go raw with `TCSANOW`. `TCSAFLUSH` discards what was typed while it
     started (#580).
6. **Renaming the attach process with `PR_SET_NAME` would work and is rejected** (#581). herdr
   would label it, run the harness's manifest over the screen, and let its rules outrank Kampr's
   reports. That brings back #405's hold and makes `HerdrProvider` misread the pane.

7. **What owning the PTY gives Claude Code depends on its renderer** (#587, #588).
   - Under **fullscreen** (`"tui": "fullscreen"`, the operator's setting) Claude takes the
     alternate screen and nothing it draws ever enters a scrollback, so a keeper's log holds a
     replayable screen but no history. Claude's past is its own virtual scroll and the transcript,
     as today (ADR 0005). A resize is a clear and one repaint of the visible screen.
   - Under **classic** (`"tui": "default"`) a finished turn enters the main screen's scrollback
     once and is never reprinted, so the keeper's log does give a complete, styled Claude history
     with no stitching. But Claude wraps every line itself, at the width the PTY had when it wrote
     it. **Per-connection reflow (§3) does nothing for Claude**: a phone watching a 100-column
     session gets 100-column rows either way. Reflow still pays for shells, logs and build output.
   - Either way a streamed reply costs about 13–14× its text in bytes, and Claude asks the terminal
     four questions at boot (DA1, XTVERSION, the kitty keyboard query, `OSC 7501`). An unhosted
     session has no terminal to answer them, so `kampr-native` answers as a good terminal does:
     DA1 and DA2 as herdr does, XTVERSION as `kampr`, DSR, DECRQM, `18t`, `OSC 10`/`11`/`12`/`4`
     with the colours the clients paint, the kitty query with the flags the program pushed, and
     `OSC 7501` (§2). A hosted session is answered by its host terminal, and its 7501 by the attach
     (§4). Answered, Claude pushes kitty flags 5
     and draws exactly what it drew unanswered (#594, #595); it asks no colour question. It takes
     legacy bytes under those flags, and kitty keys without them (#596), and herdr 0.9.5 tracks a
     pane's pushes and encodes its keys to match (#597), so a session can move between unhosted
     and a desk either way. Named keys from a phone are encoded for the flags pushed, which makes
     shift+Enter a newline in Claude's composer.
8. **The keeper split works** (#589, #590). A re-exec upgrade is invisible to the program, and with
   the master in systemd's fd store a SIGKILLed keeper comes back with the session intact. The
   control run without the store hung the child up. `research/probe/keeper.py` (about 150 lines,
   with the probe driver) is the whole of §1 except `state` and the `fd` socket.

The first holder stand-in (`research/probe/holder.py`, about 120 lines) served clients over a
socket from an in-memory log. It was enough to show that the lifecycle works against a real herdr;
§1 replaces it because a socket-serving holder cannot be upgraded or crash without taking every
session with it.

## Phases

0. ~~The probes above, as rows.~~ Done: #575–#593.
1. ~~`kampr-keep` with the fd store, re-exec and `state`; terminal-query answers; `NativeProvider`;
   `native.create` from the phone.~~ Done. The emulator, first `vt100`, is `alacritty_terminal`
   inside `kampr-native` (§2). The keeper's tests include a SIGKILL mid-stream under systemd and a
   `USR1` upgrade; the node's include a restart rebuilding the same grid from the log.
2. ~~Fold `FleetProvider` onto the keeper.~~ Done. A fleet run survives a node restart still
   asking, its screen rebuilt and its answers reaching it; the fleet wire face is unchanged.
3. ~~`kampr attach`, in a plain terminal.~~ Done, with `kampr new` and `kampr ls`.
4. ~~Hosting in herdr: the `hosts` file, the reports, dedupe in the registry, `native.desk`.~~ Done,
   live-tested against herdr 0.9.5 including re-hosting after a stop and a SIGKILL. A restored host
   pane on 0.9.1 is left detached rather than typed into, and stays that way by decision
   (2026-10-23): five of the six hosts run 0.9.5, where herdr re-hosts by itself, and the cure for
   the sixth is upgrading its herdr. Typing an attach into a restored shell is a write into a pane
   the node did not spawn, which ADR 0015 would have to be amended for, to serve a herdr nobody runs.
5. ~~Per-connection reflow on the wire, for everything but Claude (#588).~~ Done (§3). The phone
   asks for it by default on a native shell narrower than itself, with a switch on the size panel;
   the terminal client asks by the same rule for a shell wider than its box, unless it holds the
   pane at its own size.
6. ~~Harness status pushed into the node.~~ Done without hooks: Claude Code's own `OSC 7501`
   channel (§2), answered by the node for an unhosted session and by `kampr attach` for a hosted
   one, live-tested against herdr 0.9.5 (#611, #612). Other harnesses keep the marker, the title
   and the output window.

Phases 1–3 are useful with no herdr on the machine at all. That is also the Android local-PTY
provider `ARCHITECTURE.md` §9 parks, with a different keeper.

## What this costs

- **A long-lived process holding every agent**, one per session. It is a new trust boundary
  (`docs/08` needs a section): same-uid only, no network, three requests (`fd`, `state`, `resize`). Upgrades are
  free (#589) only while the new keeper accepts the old one's handover, so that handover is the
  one format in Kampr that can never change. The node sends that `USR1` itself, so the release
  ritual has no keeper step.
- **A keeper crash loses the program's exit status** and the `state` reads (#590), and at most one
  read's bytes. A crash is a bug, so this is the price of a bug rather than of an upgrade.
- **Logs live in RAM.** tmpfs counts against memory, so segments are capped per session, and a
  reboot ends the sessions and their logs together.
- **The desk experience stays herdr's.** Layouts, keybindings, plugins and agent manifests are not
  rebuilt. A hosted session borrows all of them; an unhosted one has none.
- **Two kinds of pane for a while.** Found herdr panes keep every ADR and workaround they have.
  Native sessions delete those workarounds only for themselves. Whether the herdr path ever shrinks
  depends on where the operator ends up starting agents, which is the measurement this design is
  set up to make.
