# The CLI client — briefs

`kampr` as a terminal client of its own herd: herdr's shape and herdr's habits, over the mesh,
across every node at once. Split into workstreams that can run in parallel. Read
[`04-wire-protocol.md`](./04-wire-protocol.md) first — the client writes against that, never
against herdr.

The thing this is trying not to be is a worse herdr. That failure has a specific shape and it is
worth naming before any code: a client whose keymap is *nearly* right, whose mouse does nothing,
whose panes are cropped, and which cannot rename a tab. Each of those is addressed by a named
workstream below, and W0 exists because three of them are currently guesses.

What it has that herdr structurally cannot: panes from several servers on several hosts in one
window. A herdr TUI attaches to exactly one server ([ADR 0002](./adr/0002-kampr-never-resizes-a-pane.md),
consequences). That is the whole reason to build this.

## What is already true, and what is guessed

Everything below is either measured here or taken from herdr's own docs under `research/`. Anything
not on this list is a guess and belongs in W0 before it belongs in code.

| Fact | Source | Confidence |
|---|---|---|
| Bare `herdr` launches or attaches the TUI; it is not a discovery command | `research/herdr-skill.md` | herdr docs |
| herdr's prefix is `ctrl+b`; `ctrl+b q` detaches; `ctrl+b ctrl+b` sends a literal | probe #290 | **measured, #290** |
| `herdr --remote` uses the **local** keybindings by default, `--remote-keybindings server` to switch | `research/herdr-doc-persistence-remote.mdx:49` | herdr docs |
| `--remote` is an SSH-hosted TUI client, not a socket API and not `control` | `01-implementation-findings.md` §1.8 | herdr docs |
| The rest of herdr's keymap | probe #289 | **measured, #289** |
| Copy mode (`prefix+[`) and resize mode (`prefix+r`) are **modal** — a second keymap, no prefix | probe #290 | **measured, #290** |
| `CSI 8;rows;cols t` is ignored by ghostty **and** kitty; honoured, unclamped, by konsole | probe #291 | **measured, #291** |
| `CSI 18t`/`14t`/`16t` and `CSI >0q` answer in all three, so a client can ask its size and its host | probe #291 | **measured, #291** |
| Observe frames carry **no** mouse mode, and nothing else on herdr's socket does either | probe #292 | **measured, #292** |
| Observe frames carry **no** alt-screen mode; `max_offset_from_bottom == 0` stays the only, ambiguous, hint | probe #293 | **measured, #293** |
| `control` neither refuses nor evicts an attached desk TUI — it reshapes the PTY under it, silently | probe #298 | **measured, #298** |
| `observe --cols` crops, never reflows | probe #15 | measured |
| Only `control`'s `terminal.resize` reflows, and it always claims the PTY | probes #15, #17 | measured |
| A frozen controller holds geometry forever; Kampr must time it out itself | probe #20 | measured |
| Observer and controller stream concurrently | §1.6 | measured |
| A headless session has no geometry competitor | §1.7, probes #68/#84 | measured |
| `kampr-term` tracks **only** DECSET 25 (cursor visibility) | `perform.rs:392` | read — and there is nothing else to track (#292) |
| `rename` already routes to `pane`/`tab`/`workspace.rename` by target type | `manage.rs:203` | shipped |
| `pane.process_info` gives `{pid, name, argv, cmdline, cwd}` per pipeline member — but names only `bash` under ble.sh | probe #297 | **measured, #297** |
| `pane.report_metadata` works: the title reaches the pane border at the desk and survives pane updates | probe #294 | **measured, #294** |
| Its arbitration is last-writer-wins over a per-source table; `ok` does not mean applied | probe #295 | **measured, #295** |
| `agent.view.set` sorts and filters herdr's agents **sidebar** by a Kampr-reported token — `agent.list` is unaffected | probe #296 | **measured, #296** |
| The whole herdr management surface is already on the wire | `04-wire-protocol.md` "Herd management" | shipped |
| Attachment bytes cross the mesh, chunked on a bulk lane | probes #257, #258 | measured |

---

## W0 — Probes

**Owns** `docs/03-probe-log.md` rows. Blocks W2, W3 and W5. Append rows, never renumber; propose
unnumbered and let one writer assign if two land at once.

- **herdr's full keymap.** `ctrl+b` and `ctrl+b q` are all this repo knows. Drive `herdr --help`,
  the command groups, and the default `config.toml` keybindings. One row, a table of binds. Every
  later workstream reads it instead of guessing.
- **XTWINOPS.** Does `CSI 8 ; rows ; cols t` resize the window in kitty, wezterm, foot, xterm,
  alacritty, and the operator's actual terminal? Record per-terminal, with versions. This is W3's
  whole fit strategy and it must not be assumed.
- **Mouse reporting.** Whether herdr's `observe` frames carry DECSET 1000/1002/1006 at all, and what
  a pane's program has enabled. `kampr-term` ignores every private mode but 25, so the node cannot
  currently tell a client whether a click means anything.
- **Alt screen.** Whether 1049 appears in observe frames. `scrollback_rows == 0` is today's only
  hint and it is ambiguous — "no ring (alt screen) **or** unsafe to read".
- **Does `control` evict a desk TUI?** #21's refusal says *"already has an attached client"* and
  every probe in #17–#21 ran headless. Gates W7 and nothing else. Do it last.

**Done when** every table row above says "measured" or the guess is written down as one.

---

## W1 — `kampr` the command, and how it finds a herd

**Owns** `crates/kampr-cli/src/main.rs`, a new `crates/kampr-client`.

`Cli.command` becomes `Option<Command>`. No subcommand launches the TUI, exactly as bare `herdr`
does. Nothing existing changes behaviour.

Resolution order, first hit wins:

1. **A node on this machine.** `Dirs::config()` resolves and the node answers `/healthz`. The CLI
   is running as the node's own user with read access to its state, so it mints itself a device
   through `kampr_auth::store::{create_device, mint_token}` and connects to the local origin.
2. **A saved client profile** in the client config — a token from a previous pair.
3. **Neither.** Print how to pair and exit non-zero. Do not prompt.

**The self-minted device is a real device, named and revocable.** `cli@<hostname>`, listed by
`kampr setup`, revoked like any other, written to the audit log at creation. It is not a bypass and
there is no code path that authenticates without a token. `mesh.rs::hub_device` is the precedent —
a minted device for a principal that is not a person.

This grants nothing that was not already granted — but **not for the reason the first draft of this
document gave.** It claimed *"anyone who can read the node's state directory can already read every
device token in it"*, and that is **false**: `mint_token` stores `secret::digest(token)`, so read
access yields SHA-256 digests and no tokens at all. The true claim is about **write** access —
anything that can write `kampr.db` can insert its own device row and token digest without going
near this code, so minting requires exactly the access forging already requires. That is what
`docs/08-threat-model.md` §5 says, and it is what a reviewer should be shown.

`kampr` on a machine with **no** node but a saved profile is the ordinary remote case, and it is how
a laptop drives the herd. `kampr` on a hub shows the hub's peers because the herd already contains
them; there is no second code path for "remote".

The reusable client — dial, token, `hello`/`herd` state, reconnect — lives in `kampr-client` so the
TUI is not the only thing that can ever use it. **Do not refactor `kampr-mesh` to get there.**
`shadow.rs` is already public and already does the decoding: `StyleTable::absorb`, `decode_row`,
`Shadow::{reset, patch, full}`, `History`. Depend on it.

**Done when** `kampr` on any of the three machines opens on that machine's herd with no arguments
and no prompt, and `kampr setup` lists the device it minted.

---

## W2 — The TUI shell

**Owns** the chrome, the input router, the two views. Reads W0's keymap rows (#289, #290).

**The layout is herdr's, with one structural change.** Left sidebar of two stacked sections —
`spaces` over `agents` — then a tab strip, then bordered panes with inset titles and an accent
border on the focused one. The change is that `spaces` is **grouped by node**: a node header line
carrying its name, status dot and rtt, with that node's workspaces indented under it and the git
branch as the dim subtitle herdr already puts there.

```
 spaces                      │   opencode │ ❰ herdr ❱ │ +
 comingclean      ● local    │ ┌─ claude ─────────────┐┌─ bun ──────────────┐
   ▸ herdr                   │ │ Claude Code v2.1.198 ││ ~/Projects/herdr   │
     master                  │ │                      ││ master             │
   ▸ web-dashboard           │ │ ❯ make the hero mock ││ ❯ bun run dev      │
     feat/usage-charts       │ │                      ││ astro v5.18.1 ready│
 workbox           ● 41ms    │ │ ⠴ Baking… 14m · esc  ││ ❯ watching for     │
   ▸ data-pipeline           │ │ ❯ █                  ││   file changes…    │
     backfill/events-v2      │ ├──────────────────────┤│                    │
 laptop          ○ offline   │ │ ~/…/website master   ││                    │
     4 panes · seen 13:44    │ └──────────────────────┘└────────────────────┘
 agents           grouped    │
 ◐ herdr                     │ comingclean/herdr:1 · claude · 93×40 · ⇱ 120→93
   working · claude          │
 ⚑ web-dashboard             │ ^b w herd  ^b c new  ^b , rename  ^b q detach
   blocked · claude          │
```

**Both sidebar sections already exist as models — render them, do not design them.**

- `spaces` is `Herd.groups()` (`model/Herd.kt:48`): panes grouped by node, local node first. It is
  the data model for this section already.
- `agents` is `TriageItem` / `TriageList`, which [`02-roadmap.md`](./02-roadmap.md) P6.9 calls the
  one Collie idea worth stealing wholesale. Status is `idle · working · blocked · done · unknown`
  straight off `herd.panes[].agent_status`.
- **That section is sorted by priority, not grouped** — working and blocked at the top, done and
  idle below. The word in herdr's own header is a sort mode, and herdr has an API for it:
  `agent.view.set {source, filter, sort[], label}`, where a sort field is a builtin **or**
  `{token: "..."}` and the filter is a full boolean tree. Sort locally to start with. Driving
  herdr's own view is W9's business, not this one's.
- The dots are **existing tokens** — `--working`, `--blocked`, `--idle`, `--done` in
  [`design/Tokens`](./design/), beside a full 16-colour ANSI ramp per theme. Read them; do not pick
  new colours, or the three clients stop agreeing. The `phosphor` theme is already all-monospace
  with 2px radii and is the closest thing to a terminal-native variant.

**An offline node keeps its row**, with its pane count and last-seen — `laptop ○ offline` above.
Panes are not dropped for an outage (#70), so the rule is mark stale and keep the cached grids, and
`nodes[].detail` is the operator-readable reason to show beside it. herdr has no equivalent state
because it only ever talks to one server; this is the sidebar earning the whole project.

The **herd view** is a second binding away: every node, every workspace, every pane, one screen,
with `⚑ blocked` first. It is the triage screen you cannot get at a desk.

**Two panes side by side is Kampr's own mosaic, not a herdr split.** It is a client-side arrangement
of independent `observe` streams that may come from different sessions on different hosts, and it
*"needs no protocol support beyond watching several panes at once"*. Do not confuse it with
`manage`'s `pane.split`, which changes herdr's layout for everyone at the desk.

**`prefs` arrives unasked as the third greeting frame**, after `hello` and `herd`, on every
connection and even when nothing is stored. The CLI stores its per-pane view choice there so it
follows the operator between machines. A write is a **merge**, `null` removes a key, and the first
`prefs` on a socket is not the answer to your own write.

**Re-draw on `role`.** A demotion or promotion lands on the open connection as `{"t":"role"}` and is
not a second `hello` — a client that re-ran its greeting would throw away its herd over a permission
change. Gate write affordances on it, not on the role you were greeted with.

The input router is the whole risk in this workstream. Rules:

- **The keymap is the client's, not the node's.** herdr made this choice for `--remote` and gave the
  reason: local muscle memory beats remote config. Copy it, and copy the escape hatch — a flag to
  take the node's instead.
- **Prefix is `ctrl+b`, and `ctrl+b ctrl+b` sends a literal `ctrl+b`.** Anything not a binding after
  the prefix, and everything outside it, goes to the pane.
- **Input goes over `input`, never a terminal stream.** `text` is preferred; `keys` only for what
  herdr's grammar accepts (probe #7). Home, End, PgUp, PgDn, Insert and Delete are *not* in that
  grammar and go as escape sequences through `text` (probes #8/#9).
- **A readonly device draws no write affordances**, and re-draws on the `role` frame mid-connection.

**Done when** a herdr user's fingers work without being told anything, and the keymap row is what
proves it.

---

## W3 — The grid, and making it fit

**Owns** cell → ANSI, damage tracking, and the fit ladder. Reads W0's XTWINOPS row (#291).

Rendering is `Vec<Cell>` out of `Shadow` and into ANSI. The correctness traps are all in the wire
doc and none of them are optional: count **code points, not UTF-16 units**; the column after a wide
glyph belongs to that glyph; a cell's text is its base in `x` followed by its marks in `m`; `links`
on `grid.reset` **replaces** and on `grid.patch` **appends**, and getting that backwards resolves
links to the wrong URL rather than failing visibly.

The fit ladder, in order, each falling through to the next:

1. **The terminal is wide enough.** Common — a headless PTY measured 93 columns (#68) and a modern
   terminal is usually wider. Draw it and stop.
2. **Ask the terminal to resize itself, and check whether it did.** XTWINOPS. **This is not the
   cheap inversion the first draft of this document called it, because on this desk it does not
   fire at all**: ghostty 1.3.1 — the operator's own terminal — ignores `CSI 8;rows;cols t`
   outright, and so does kitty 0.48.2 (#291). Only konsole honoured it, and it honoured it *too*
   well: asked for 400x900 it gave 400x900 on a 2560x1440 display, so an unguarded rung 2 hands the
   operator a window they can see a slice of.

   So the rung is **self-answering and self-clamping**, in this order, and every step is measured:
   name the host with `CSI >0q` (all three answer it); compute the largest grid the display can
   actually hold from `CSI 14t` divided by `CSI 16t` (**not** from `TIOCGWINSZ`, whose pixel fields
   go stale in konsole while `14t` stays honest); refuse the request ourselves if the pane is wider
   than that; write the request; re-read `TIOCGWINSZ`; and treat anything past **50 ms** as a
   refusal — konsole landed in under 5 ms with one SIGWINCH, so the deadline is ten times the only
   measurement there is. A terminal nobody has measured gets probed the same way and answers for
   itself.
3. **Crop and pan.** What the phone does. Always available, never wrong, sometimes annoying — and
   on this desktop it is **the path, not the fallback**, because rung 2 falls through on two of the
   three emulators here. Engineer it accordingly: this rung is where the felt quality of the client
   is decided.

**Never derive geometry.** `cols` is absent until something has measured it and a client shows
nothing rather than falling back to the rect, which is a width no row was ever wrapped at.

Client-side re-wrapping is **out of scope and should not be attempted.** The rows arrive already
wrapped at the PTY width; re-wrapping them is not what the program would have drawn, and the signal
that would say which panes it would ruin **does not exist**: #293 looked for the alt screen in the
observe stream and found neither `?1049` nor `?47`, and no other question herdr answers separates a
pane on the alt screen from a pane that has simply never scrolled. That ambiguity is permanent, so
this stays closed rather than waiting on a row that is now known not to be coming.

**Done when** a 171-column zoomed pane is legible in an 80-column terminal by some rung of that
ladder, and the ladder says which one it used — including saying *rung 2 was refused by this
terminal*, which on ghostty is the answer every time.

---

## W4 — Management, so it is not a viewer

**Owns** the `manage` surface in the TUI. Nothing new on the wire.

The entire herdr management surface already ships: `workspace.create`, `tab.create`, `pane.split`,
`pane.zoom`, `rename`, `close`, `focus`, `agent.start`, `worktree.create`/`open`,
`layout.export`/`apply`, `session.create`/`stop`. Bind them and render the acks.

- Gate every one on `hello.caps.manage`; hide what a node does not claim.
- **Wait for the `herd.patch`.** Never optimistically mutate the model — the node is authoritative.
- Watch `ok` on the `managed` ack, not just its arrival. Every refusal is acked, including
  `not_writer` and `bad_request`, and a client that waits on arrival alone hangs on a refusal.
- Use `rid`, because several ops will be in flight from a keyboard-driven client.
- `tab.rename`, `tab.close` and `tab.focus` need `tab_id`, which a pane id does not carry. Read it
  off the pane entry.
- **Say what a split will do before doing it.** It changes geometry for everyone at the desk too.
  That is not a violation of ADR 0002 — the invariant is about side effects of *viewing* — but it is
  a surprise if unannounced.

**Done when** renaming a tab, splitting a pane and starting an agent all work from the keyboard, on
a peer's node, without touching a browser.

---

## W5 — Mouse

**Owns** the client's own hit testing, and a per-pane passthrough toggle. **Nothing on the wire, and
nothing in `kampr-term`** — the first draft of this section proposed both and #292 removed the
premise from under them.

**What #292 settled, and why it is structural rather than a gap.** herdr's observe frames carry
exactly four private modes — `?2026h/l` and `?25h/l` — in 24 of 24 frames. DECSET **1000, 1002,
1003 and 1006 never appear**, set or reset, however the pane's program asks for them. That is not
the encoder dropping them: observe emits *grid state*, not a byte replay (#23/#25), so a mode change
is consumed by herdr's own emulator and has nothing left to be re-emitted as. Nor is it anywhere
else on the socket — `pane.get`, `pane.list` and `session.snapshot` carry no such field, and
`pane.graphics.info`'s `pixel_mouse` is a decoy that reads constant `true` and describes the *host*
client's capability, not the pane's program.

So **teaching `kampr-term` to track 1000/1002/1006 would be teaching it to parse a stream that never
contains them**, and the `grid.reset` field that was to carry the answer would have nothing to say.
Both are cut. `herdr --remote` has a mouse because it *is* herdr's own TUI on the PTY; a cell-grid
client is not that, and no additive wire change reaches it.

Three halves, then, and the first is most of the felt benefit:

- **Clicks on Kampr's own chrome are client-side and free** — a tab, a node or pane row in the
  sidebar, a pane in the herd view, the scrollbar, a pending prompt's options. This is the bulk of
  the workstream and it is unblocked. Focus follows the click; a click on a pane body focuses that
  pane without sending anything into it.
- **Selection, and the copy that follows it.** Drag over the grid selects; the copied text is the
  **logical** text, not the painted grid — trailing padding stripped, soft-wrapped rows joined —
  because a path or a URL copied with a newline through the middle of it is worse than not copying.
  Same rule the wire doc states for the phone. Clicking a cell whose `link` resolves opens the OSC 8
  URI; a *detected* bare URL is offered, never auto-navigated, because pane output is
  attacker-influenceable.
- **Clicks *into* a pane, behind an explicit per-pane toggle that is off by default.** #9 says a
  client can send an SGR mouse report as raw bytes through `pane.send_text`; the only thing missing
  is knowing when that is safe, and #292 says nothing will ever tell us. **So the operator tells
  us.** The toggle is remembered per pane in `prefs`, so it follows them between machines, and the
  status line says plainly when a pane is passing the mouse through. Encode SGR 1006 with 1002 drag
  reporting — the shape every mouse-aware program of the last decade accepts — and send it as
  `text`.

  **Offer the toggle, never flip it.** `pane.process_info` names the foreground process (#297), so a
  pane running a known mouse-aware program can be *suggested* — but #297 also measured that under
  ble.sh, which is on every interactive shell on the operator's own machine, `process_info` names
  only `bash`. A heuristic that fails open there would be typing into a shell, which is exactly the
  thing this rule exists to prevent.

Do not send mouse escape sequences to a pane that has not asked for them. That is how you type
`[<0;10;5M` into somebody's shell — and after #292, *asked* can only ever mean the operator asked.

**Done when** clicking a tab focuses it, dragging the grid selects text that pastes back as one
logical line, and a pane the operator has armed drives `vim` the way it does at the desk.

---

## W6 — Images

**Owns** the attachment fetch and the inline renderer.

`att` headers ride on `md` blocks in `convo`/`convo.turn`; the bytes come from
`GET /api/attachment/{node}/{local}/{id}` with the device token, 8 MiB ceiling. Render with kitty
graphics, iTerm2 inline, or sixel by terminal, and fall back to the `[image · png]` marker text that
is already in `text` — which is exactly what a client that ignores `att` shows today.

- **Cross-mesh works.** A hub asks the peer over the link it dialled and streams the answer back,
  chunked 64 KiB on a bulk lane so the pane keeps repainting during the transfer (#257).
- **A hub strips `att` when it cannot serve it** — peer offline, unknown, or too old. Absent button,
  not a dead one. Render nothing rather than a broken image.
- **Never render a `404` as a missing image.** An id names a record in a transcript and stops
  resolving when the transcript is rewritten; that is expected, not an error state.
- **Images drawn by a program inside a pane are out of reach**, and it is herdr's limit, not ours:
  observe coalesces to grid state rather than replaying bytes (#23). Only W0's control-stream row
  could reopen it.

**Done when** a screenshot an agent pasted on another host renders inline in the terminal.

---

## W8 — Conversation and prompts

**Owns** the conversation view and the pending strip. Independent of W3 — a different renderer over
the same socket.

This is the largest gap in the first draft of this document, and it is not optional: **an agent pane
opens on the conversation, not the grid** ([ADR 0005](./adr/0005-structure-comes-from-the-transcript.md)).
A CLI that only ever draws grids is a CLI that opens every agent pane on the wrong view.

- `convo` / `convo.turn` carry turns of `md` | `code` | `tool` | `diff` blocks. Markdown is passed
  through verbatim **for the client to render** — so tables stay tables, which is the entire reason
  the node does not pre-render it.
- **A page merges by id, and `fresh` means replace rather than merge.** Unconditional prepending was
  the older rule and is wrong for a transcript re-read: the missing turns are then the *newest*, and
  every one lands at the top of a view scrolled to the bottom, never seen. Read that section of the
  wire doc in full before writing the merge.
- `convo.load` pages backwards; `cursor` **absent** is not the same as `more: false`.
- `has_conversation` means a transcript resolves, not that this harness has an adapter, and it can
  never outrun `hello.caps.conversation`.

**`pending` is what makes the triage list pay off.** A prompt waiting on a blocked agent arrives as
`{"t":"pending", question, options[], source}` and is answered with `{"t":"answer", pane, key}`.

- **The node decides whether a submit key follows**, per harness — Claude selects on the digit
  alone, Codex needs Enter (#43). Send only the key you were offered in `options`. Never synthesise
  an Enter.
- **A prompt is cleared by the same message with `question: null`.** There is no resolved event.
- `source` is `transcript` or `screen` and **clients must not care which**.

Answering a blocked agent on another host from the sidebar is the single best thing this client can
do that a herdr at the desk cannot.

**Done when** `⚑ blocked` in the sidebar is one keystroke from reading the question and answering it.

---

## W9 — Naming that says what a pane *is*

**Owns** a template engine and its config, in `kampr-core` so every client shares it. Cross-app by
construction — this is not a CLI feature.

The complaint is real: `dir-name` is a poor identifier when six panes share a directory. The wanted
shape is a template — `{workspace}`, `{cwd}`, `{cmd}`, `{agent}`, `{status}`, `{branch}` — resolving
to things like `kampr (cargo test)` rather than `kampr`.

**Most of the hard input already exists.** `pane.process_info` returns
`foreground_processes[] { pid, name, argv }`, and `kampr-core` **already calls it** on every pane to
find the agent harness (`herdr_provider.rs:540`). The running command is on the node today and
simply is not on the wire. That is the cheap half.

Template inputs, and their honest status:

| Token | Source | Status |
|---|---|---|
| `{workspace}` `{tab}` `{cwd}` `{label}` | `PaneEntry` | on the wire today |
| `{agent}` `{status}` | `PaneEntry` | on the wire today |
| `{cmd}` `{argv}` | `ForegroundProcess.{name, argv, cmdline, cwd}` | **on the node, not on the wire — and blank under ble.sh (#297)** |
| `{branch}` | — | not modelled anywhere yet |
| `{last_cmd}` | — | **no source.** Needs scrollback parsing or shell integration. Do not promise it |

**The delivery mechanism is the find worth acting on, and it is measured now.** #294 answered the
question this section was gated on: **`pane.report_metadata` is not another #75.** A title Kampr
computes draws on the pane's border at the desk (`+- zulu (cargo test) -+`), arrives over the API as
a new `title` key on `pane.get`/`pane.list`/`agent.list`, and is durable — it survived pane output,
an `OSC 0` title change from the program, a `pane.rename` and a re-focus, each of which lands on a
*different* field. `display_agent` replaces the agent's name in herdr's agents sidebar, and reported
`tokens` render there as `$name` rows. #296 closes the other half: a token Kampr reports really does
become the field herdr's own agents view **sorts and filters** on, with `label` replacing the
sort-mode word in the header.

**Five measured constraints, and every one of them will bite an implementation that assumes the
schema.**

- **`ok` means *well-formed*, not *applied* (#295).** The only honest confirmation is to read
  `pane.get` back. A `seq` older than the one that source last sent is dropped **silently** and
  still answered `ok`.
- **Arbitration is last-writer-wins over a per-source table, not a priority (#295).** Each source
  keeps its own record. `clear_title` from the source that was showing does not blank the pane — it
  *reveals* another source's title. `ttl_ms` is an expiry of one record and falls back to the next,
  never a clear.
- **`applies_to_source` is a silent conditional that returns `ok` when it does not fire (#295)** —
  and worse, a guard that misses **withdraws the reporter** rather than leaving the previous value
  alone. Do not use it without reading the pane back.
- **`state_labels` is a closed vocabulary** — `idle`/`working`/`blocked`/`unknown`/`done`, and each
  call **replaces** the whole map rather than merging it (#294).
- **`title` is not a sidebar row builtin, and putting it in `rows` makes herdr reject the entire
  config** (#294). What an operator sees past the pane border is their own `rows` config, not
  Kampr's to choose. `agent.list` is unaffected by `agent.view.set`, so a client **cannot read back
  the view it just set** and must sort locally regardless (#296) — which is what W2 already says.

**`{cmd}` is real but not universal, and the exception is the operator's own machine (#297).**
`pane.process_info` names the job, its argv and every member of a pipeline on a plain shell —
`sleep 9 | cat` comes back as both — and the response is richer than the schema says:
`{pid, name, argv, cmdline, cwd}`, with `cmdline` pre-joined. But ble.sh keeps the command in the
shell's own process group, so on a machine sourcing it `process_info` reports `bash` and
`foreground_process_group_id` never leaves `shell_pid`. **A `{cmd}` with no fallback therefore
resolves to nothing exactly where this workstream's complaint came from**, so the template must
degrade — to `{workspace}`, then `{cwd}` — rather than render an empty name.

Ordering: the probe is **done** (#294–#297). Next is the template engine and `{cmd}` on the wire as
a new optional `PaneEntry` field (additive), then reporting back to herdr **behind a setting that is
off by default** — writing names into somebody's herdr session is a side effect of viewing, and this
project has a rule about those.

**Done when** six panes in one directory are told apart at a glance, in the CLI *and* at the desk.

---

## W7 — Resize, if we want it — deferred

**Owns** a successor to ADR 0002. **W0's control-eviction row has landed and it is worse than a
refusal: #298 measured that `control` neither refuses nor evicts an attached desk TUI — it reshapes
the PTY underneath one that goes on drawing the old box.** herdr's layout rect never moved, a
69-column line came back whole from the API and appeared on the desk cropped at 49, and no error was
raised anywhere. #21's *"already has an attached client"* is about a second **controller of the same
terminal stream**, not about a desk at all; every probe in #17-#21 having run headless is exactly
why that read survived. This makes the headless-only scope below **non-negotiable rather than merely
prudent**, and it is ADR 0002's invariant with a visible victim. Do not start this until
W1–W3 have shipped and the fit ladder has been lived with, because rung 2 may make it unnecessary.

Scope, if it happens: `terminal.resize` on panes in **headless sessions only**, where §1.7 says
there is no geometry competitor and #68/#84 say the PTY never followed the rect anyway. The node
spawns one control child per pane and is itself the sole controller, so herdr's arbitration never
fires and there is no lease protocol between viewers — last writer wins, which is herdr's own answer
(#31). The one thing that must be built is the timeout for a wedged controller, which #20 measured
and §1.6 names outright: *"This is the one case Kampr must time out itself."*

`terminal.scroll` arrives with it, and would undo most of [ADR 0004](./adr/0004-scrollback-is-stitched-and-a-gap-discards.md)'s
compromise chain for controlled panes. That is the real prize here, not the reflow.

ADR 0002 says the resize path is **structural, not policy** — *"the code path does not exist"*. Adding
one for headless panes changes that claim and needs a successor ADR naming exactly which panes and
why. The wire stays additive: new `t` values, ignored by every phone already in the field.

---

## Working concurrently

Read this before spawning anything. Most of these workstreams touch the same crate, and three of
them share a resource that does not tolerate two writers.

### Where the code goes — decided, do not re-decide

Two new crates. A stream that invents a third has diverged.

```
crates/kampr-client/          W1 — dial, token store, hello/herd state, reconnect. No TUI.
crates/kampr-tui/
  src/lib.rs                  W2 owns. Module declarations and the event loop.
  src/app.rs  sidebar.rs      W2 — chrome, the two views
  src/input.rs  keymap.rs     W2 — prefix routing, binds
  src/render/grid.rs  fit.rs  W3 — cell → ANSI, damage, the fit ladder
  src/manage.rs               W4
  src/mouse.rs                W5
  src/image.rs                W6
  src/convo.rs                W8
crates/kampr-core/src/naming.rs   W9 — shared with every client, so not in the TUI crate
```

`crates/kampr-cli/src/main.rs` gains `Option<Command>` and a call into `kampr-tui`. **W1 owns that
file**; nobody else edits it.

### What can actually run at once

The honest dependency graph, which is not "nine agents at once":

- **Fully concurrent, now:** W0 and W1. They share nothing.
- **W2 and W3 are one unit.** They co-own the frame — the event loop calls the renderer every tick.
  Split across two agents they will fight over `lib.rs` and the draw path. One agent, or strictly
  sequenced.
- **Concurrent once W2 has landed the module skeleton:** W4, W5, W6, W8. Each is a self-contained
  module plus one line in `lib.rs`, so the only contention is that line.
- **W9 is concurrent with everything** — different crate, and its first task is a probe.
- **W7 is deferred** and is last in this file for that reason. Do not start it.

### Three shared resources that need one writer

1. **`docs/03-probe-log.md` is append-only, gap-free, and numbers are permanent identifiers that
   code cites.** Two agents appending at once will collide or, worse, both claim a number. Propose
   rows **unnumbered** and let a single writer assign. `grep -c` a number before citing it.
2. **A live herdr is a single machine-wide resource.** A node serves *every* herdr session it can
   find (#97), and live tests are sensitive to machine load. Two agents running `live.rs` or driving
   probes at the same time will interfere and produce failures that look like regressions. Serialise
   anything that touches a real herdr, use a throwaway named session, and tear it down.
3. **`crates/kampr-tui/src/lib.rs`** — W2 owns it. Other streams add their module and say so rather
   than racing to wire it.

### Verifying

The four gates from [`CLAUDE.md`](../CLAUDE.md), all of which must be clean:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd client && env -u GRADLE_HOME ./gradlew check     # W9 only; it touches shared client code
```

`env -u GRADLE_HOME` is not optional — a `GRADLE_HOME` on this machine silently overrides the
wrapper (#67). The Rust suite includes live tests that need a real `herdr` on PATH; they skip loudly
rather than pass quietly, and a failure should be re-run alone before it is reported as a
regression.

---

## Cross-cutting — true in every workstream

These are wire rules, not features, and a client that gets one wrong looks broken in a way no test
in its own workstream will catch.

- **Reconnect renders the cached grid, marked stale, and swaps on `grid.reset`. No spinner.** A full
  grid is ~3 KB and herdr coalesces bursts to end state (#23/#25), so there is never a backlog. A
  node that has gone offline keeps its panes, because a herdr restart keeps them (#70).
- **`error.code` is an open string.** An unrecognised code must render its `message`, never fail.
  This is how a client older than a code still shows the diagnosis, and a hub forwards a peer's
  codes verbatim, so a newer peer's code will reach you.
- **`stream_unavailable` is not an outage** — the node is answering and only the frames are missing.
  It arrives once per fault, on the edge, and is taken down by the herd entry's `detail` clearing,
  not by any frame. Do not raise a strip for ever behind a supervisor that retries for ever.
- **Gate every affordance on `hello.caps`**, and hide rather than disable what a node does not
  claim. A button that cannot work must be absent (findings §3.7).
- **Never optimistically mutate the herd model.** Wait for the `herd.patch`; the node is
  authoritative. Watch `ok` on a `managed` ack, not its arrival.
- **The wire is additive only.** Anything this document adds — `{cmd}` on `PaneEntry`, a mouse-mode
  field on `grid.reset` — is a new optional field or a new `t`. Nothing reinterprets an existing
  field, because older clients are installed on real phones.

---

## Order

W0 first and in parallel with W1 — the probes block the workstreams that guess, and W1 blocks
everything that connects. Then W2 and W3 together, which is the first thing worth using. Then W4
and W8, which are what stop it being a viewer: W4 is management, W8 is the conversation an agent
pane opens on and the prompt that answers a blocked one. W5, W6 and W9 are independent and can land
whenever, though W9 wants its probe row early because its answer may be "herdr will not let us".
W7 last, or never.

**This document is a starting point and is expected to be wrong in places.** It is built from what
is measured today plus a reading of herdr's schema, and the schema-only rows are the ones most
likely to move. Revise it as the probes land rather than treating it as settled.

Every workstream is TDD at integration level, and `live.rs` is the honest level for anything that
touches herdr. Note what #265 established: **`live.rs` runs headless**, so a test there for anything
geometry-shaped drives an op that cannot move a PTY and would go green with the whole path deleted.
W7 needs a real herdr forked under a PTY with every `HERDR_*` variable stripped from the child, the
way #265 was driven. That harness does not exist yet and building it is most of W7's cost.

Prove each fix is load-bearing by reverting it and watching the test fail. A test that still passes
with the defect restored is a harness that was never the app (#191).
