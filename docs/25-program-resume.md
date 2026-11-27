# Program resume: adopting OSC 7577

Rex also defined [the Program Resume Protocol](https://www.superlogical.com/rex/docs/build/program-resume)
(`OSC 7577`): a program tells the terminal the command that brings it back (`argv`, optionally `cwd`) and
the terminal may run it after a restart. Nothing we have seen emits it yet, so this is built for when
tools adopt it, but it is also the right shape for a gap we already have (below).

## What it is, and what it is not

The program owns its state (nvim writes a session file, Claude has its transcript). The protocol carries
only the *pointer*: `nvim -S /path/session.vim`. The terminal persists the pointer and re-executes it. It
survives a reboot exactly when the terminal stores records on disk and the program's state is on disk.

Authentication is the part to copy carefully. The terminal puts a random `OSC7577_TOKEN` in the first
process's environment on each pty. Every report ends with `mac=sha256(body ":" token)` and carries `pid`
and `time` (within 60 s of the terminal's clock, and not older than the last accepted report for that
pid). Without the token, a `cat`-ed file or anything printed over ssh cannot register a command.
Same-user processes are trusted, since they can read the environment; the spec says so plainly.

## Why we want it: the reboot gap

A keeper's directory is on tmpfs, deliberately: "a reboot ends the sessions and their logs together"
(docs/17). Nothing in the node remembers what a session was *running*, so after a reboot a native
session is simply gone. Separately, herdr persists a hosted pane's `resume_argv` (docs/17 §2, stored as
`agent_resume` in `~/.config/herdr/session.json`) and runs it on restore: **`kampr attach <id>` for an id
whose keeper died with the boot.** That is the second reported issue (below).

## Design

### Token

Each keeper session gets a token at spawn: random, 128 bits in [A-Za-z0-9_-], written to the session
directory as `resume.token` (0600, on tmpfs like the rest) and exported as `OSC7577_TOKEN` in the child's
environment. It is never inherited from the node's environment and never copied into another session's
`spec.json` (session-scoped variables are already not copied; a test proves one session cannot read
another's token).

Docker shell sessions (docs/19) get **no token**: the resumed command would run on the host as the
operator, for a program that was inside a container.

### Parsing and verification

`kampr-core` gets a pure `resume` module: strict parse (a malformed pair, repeated key, missing required
key, over-limit value, bad base64 or UTF-8, control character: reject the *whole* report), the `mac`,
`time` and `pid` checks, and `need=` (reject if we do not implement a named key). A rejection after a
valid mac and time removes that owner's records, as the spec says. The emulator in `kampr-native` routes
`OSC 7577` to it.

The token is read from the session directory by whoever parses (the node, or `kampr attach`). A replay
after a node restart would re-read reports whose `time` is long past, so **the time window applies when a
report is first read live**, not on replay: the first live reader accepts it and writes the record with
its log offset, and replays skip offsets at or below the stored one.

`pid`: alive, same uid, and a descendant of the keeper's child (`kampr-proc` answers this on Linux and
macOS). Remember the owner's start time so a recycled pid counts as gone.

### Record store

`<state dir>/resume/<session id>.json`: the node's persistent state directory, **not** tmpfs, 0600 (the
spec: a record is as sensitive as a shell startup file). One record per owner pid, written atomically on
every accept and removal. Cleared when a `clear` arrives, when the owner exits other than because the
keeper ended (with a second or two of grace at shutdown), and when the session is closed on purpose.

### Restoring

After a boot the node finds records whose keeper is gone. It does **not** run them by default: the spec's
threat model says to give the user control, and a boot is when the operator is not looking.

- `[native] resume = "ask"` (default): the herd lists them as a group, **Resume after reboot**, each with
  its program, cwd and command (escaped for display, as the spec requires). The operator's tap is the
  request. `"auto"` runs them at node start; `"off"` stores nothing.
- Running one starts a new keeper session with `argv` as a list, never parsed by a shell (a login shell
  only to give the program its usual environment, each element passed as data), in `cwd` if given. A
  `cwd` that no longer exists means the record is not resumed.
- A failed resume is never retried: a record is marked tried before it runs.
- A node running as root stores and resumes nothing.
- The resumed program is a new process with default modes and reports again, replacing the record.

### Harnesses we already know how to resume

Claude, codex and muffit sessions have an id and a documented resume command (`claude --resume <id>`,
`muffit --resume <id>`, the codex equivalent). The node can write those records itself for a native
session whose harness it recognises, with no token, as "reported by Kampr". That is the half that helps
the operator today, independent of any tool adopting 7577. It is its own phase because it needs per-harness
probes (does `--resume` need the same cwd? is a `--continue` form needed?).

## The stale attach (issue 2)

Measured on this machine: `~/.config/herdr/session.json` holds two `agent_resume` argvs of the form
`env KAMPR_KEEP_ROOT=/run/user/1000/kampr/s ~/.local/bin/kampr attach <id>`, and neither id is in the
keeper directory after the boot of 2026-11-24 22:20. herdr runs the argv on restore; `resolve()` in
`kampr-cli/src/native.rs` bails with `no session matches`.

Measured on herdr 0.9.5 (probe #694, `research/probe/stale-attach.py`): herdr **types the stored argv into
the pane's own shell**, so a failed attach is an error line above a live prompt, not a dead pane. It
keeps the argv for good, through every later restore. Nothing in the API reads it back (`pane.get` has no
such field), a report without `resume_argv` does not replace it, and `release_agent` does not clear it;
only another `resume_argv` replaces it, and an empty one is refused. A `release_agent` without a newer
`seq` is answered `ok` and dropped (#695).

Built, in order:

1. `kampr attach` on an id with no keeper: if a resume record exists, offer it (a prompt under `ask`,
   run under `auto`). Otherwise say plainly that the session ended with the machine (its log was in RAM)
   and exec the login shell instead of exiting. Both branches are built: a record is offered with its argv and cwd and a
   `[y/N]` under `ask`, run under `auto`, and a declined one stays for the next restore. The pane's shell already sits in the pane's directory, so the exec'd one
   nests inside it.
2. `resume_argv` carries a marker (`attach --restored <id>`) so a typo at a prompt still errors instead of
   silently becoming a shell; only the restore path takes the shell fallback. Panes already persisted
   without the marker keep today's error, one line above a working prompt, on every restore.
3. The restored attach clears the stale argv from inside its own pane, not the node at start: a node
   cannot find the stale panes, because the stored argv is unreadable through the API and
   `session.json` keys its panes by an internal number that only `public_pane_numbers` maps to
   `w1:p3`. The pane has `HERDR_PANE_ID` and `HERDR_SOCKET_PATH` after a restore (#592), so the
   attach reports `resume_argv: ["true"]` and releases the agent again with a newer `seq`. The next
   restore types `true`, and the pane is a plain shell.

Steps 1 and 2 need no 7577 at all and can ship first.

## Tests

1. `kampr-core` conformance table: the spec's worked example (`pid=74877:time=1795543200:clear=1` with
   token `dummy-token-replace-me`), every rejection, `need=`, `mac` last, the time window, monotonic time
   per pid.
2. Emulator: a verified report becomes a record; a forged one (wrong token) does nothing, including to an
   existing record; a `cat` of a file containing a report does nothing.
3. Store: survives a node restart; removed on close; mode 0600; not created for root.
4. Restore: with the keeper directory wiped and a record present, `ask` lists it and `auto` starts it; a
   missing `cwd` does not resume; a second boot does not rerun a failed one.
5. Stale attach: `kampr attach --restored <dead id>` on a pty ends in a shell, and without `--restored`
   errors. Written first and run red (rule 4).
6. Execution: an argv element `$(reboot)` or `a; b` arrives as one unchanged argument.

## Status (built 2026-11-27)

The token, verification, routing, store and restore are built; Kampr-written records for Claude,
codex and muffit (the second phase) are not.

- `kampr-core/src/resume.rs`: the pure parser and `mac` check; the spec's worked example verifies.
  Freshness is `Live { now }` or `Replayed`, so the 60 s window applies to the first live read only.
- A session gets a 128-bit token at `create`, as `OSC7577_TOKEN` in its child's environment (kept off
  `spec.json`, which clears its env after spawn) and `resume.token` (0600) beside its log. A Docker
  shell gets none, `[native] resume = "off"` gives none, and root gives none. The keeper drops any
  `OSC7577_TOKEN` it inherited, and `kampr-keep`'s session-scoped list drops one a new session would
  inherit from the terminal that made it.
- `kampr-native/src/resume/`: `intake.rs` reads each report against the token at its log offset
  (a replay skips offsets at or below the stored one), checks the owner is alive and below the
  session's child, remembers its start time, and removes its record when it ends other than with the
  keeper (1.5 s grace). `store.rs` writes `<state dir>/resume/<session>.json` 0600 in a 0700
  directory, atomically. `restore.rs` lists records whose session is gone and whose `cwd` exists,
  marks one tried before it runs and spends it.
- A resume runs `sh -lc 'exec "$@"' kampr-resume <argv…>`: a login shell for the environment, each
  element one argument.
- Node: `[native] resume` (`ask` default, `auto`, `off`); `native.resumable` and `native.resume`
  manage ops (docs/04); `auto` resumes at node start. `kampr attach --restored <dead id>` offers the
  session's record (`kampr-cli/src/native/resumed.rs`).
- Measured: nothing installed here writes 7577 (probes #691, #692), Claude 2.1.326 included.
- Known edge: a report written while no node is following the session (the node down, or not yet
  adopted) is read at the node's start as a replay and skips the 60 s window. It still needs the
  session's own token, so it can only be that session's own output.
- Not built: `kampr new` from the CLI gives no token (only the node's `native.create` and a resume do).

## Decisions taken

- **`[native] resume` defaults to `ask`**, not `auto`: a boot is when the operator is not looking.
- **Kampr-written records for Claude, codex and muffit are a second phase**, deferred; nothing here
  writes one.
- **Hosted herdr panes get the token through `kampr attach`; found herdr panes cannot.** That is
  rule 3's boundary too.

Under the default `ask`, a record is run from the herd's **Resume after reboot** group, the TUI's host menu, or `kampr attach --restored <dead id>`.
The herd's **Resume after reboot** group is built (2026-11-27): each node that keeps sessions is
asked `native.resumable` on connect and when it rejoins the herd; each record shows its program,
`cwd` and its command escaped for display (`displayArgv`: shell quoting, and `$'…'` escapes for
control, format and bidi characters), with **Resume** (`native.resume`, opening the new pane on the
ack) and **Dismiss** (`native.resume.dismiss`, which spends the record unrun). The TUI's host menu lists the same
records and sends both. Both clients ask only a node whose herd entry sets `resume`: 0.1.153 keeps
sessions and refuses `native.resumable`, which would be an error and an audit entry on every connect.

Still open: the second phase above.
