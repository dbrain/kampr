# Fleet runs

One command, every host, and a board that shows which one stopped to ask something.

The shape of this was decided almost entirely by measurement. The first design was a fan-out into
herdr panes with a screen scraper looking for `[Y/n]`; probes #331–#337 killed both halves of that
in an afternoon, and what is built instead is the thing that survived.

## What the probes changed

**A node cannot see whether a pane's job is waiting.** `/proc/<pid>/syscall` needs
`PTRACE_MODE_ATTACH`, and under yama `ptrace_scope=1` that is refused to any reader that is not an
ancestor of the target. The node is never an ancestor of a herdr pane — herdr forks those, and
herdr is its own service (#331). `wchan` survives and does discriminate, but it is a kernel symbol
name with no contract, and #332 removes it anyway.

**A job running as root refuses everything.** `syscall`, `wchan` and `fd/0` are all denied to an
unprivileged reader, so the node can read *literally nothing* about a `sudo pacman` — which is the
exact command this feature exists to run (#332). Not degraded; blank.

**ECHO is not a password signal on a shell pane.** ble.sh leaves an idle pane's tty with ECHO
already off, before anything asks for a secret (#333). The one termios bit that looks definitive is
inverted from the assumption on every machine the operator actually uses. It is not enough on its
own either: `vim` and `less` turn ECHO off exactly as `getpass` does, and what separates a password
prompt from a full-screen program is that a prompt is still line-based — `secret = !ECHO && ICANON`
(#340).

**And termios is trustworthy exactly where `/proc` is.** `sudo` relays for a command that has a
controlling terminal — which a fleet run always gives it — and while it does, the pty's ECHO and
ICANON describe *sudo's relay* rather than the job (#341). Reading them anyway called
`sudo pacman`'s `[Y/n]` a full-screen program and dropped it off the board. The same privilege wall
that hides the process inserts the relay that lies about the terminal, so anything inferred behind
that wall goes on the text alone.

**But a supervisor that forks the command and shares its privilege can read all of it** (#334). A
prompt is `read(2)` on fd 0; `sleep` is `clock_nanosleep`; a busy loop is `running`. And on a pty
with no *interactive* shell on it, ECHO going off is honest again (#337) — the confound #333
describes needs a shell that is reading a line, and a fleet run's shell never is. That sentence
used to read "no shell at all"; "The shell, and the three things it was not allowed to break"
below has what was measured when a shell was put there on purpose.

So the design inverted: **instrument from inside a pty we own, rather than inspect a pane from
outside.**

## What that buys, beyond detection

A fleet run is not a herdr pane at all. It has no workspace, no tab and no place in the operator's
layout, so it **cannot clutter the desk of the machine it runs on** — the grouping is structural
rather than a filter every client has to remember to apply. `kampr_core::provider::Composite` puts
the fleet provider in front of the herdr one behind the same seam the node already had, and
`herd.groups()` filters fleet panes out of the ordinary views.

It also means the exit code is `wait(2)`'s, not something scraped off a screen (#337), and that
answering is the ordinary `input` message rather than a second way to type into a terminal.

## The two layers, and which one is load-bearing

| | Answers | From | If it fails |
|---|---|---|---|
| **1** | Has it stopped for somebody? | the kernel — a read-family syscall with fd 0 in its first argument | drop to 1b |
| **1b** | …and if `/proc` is closed? | ECHO off with ICANON on (a secret, #339); else a settled unterminated line that parses as a question — **`inferred`** | the board says **quiet**, never a question |
| **2** | And what did it say? | the text it wrote and did not terminate with a newline | `Free` → a text box instead of buttons |

Rung **1b** is what makes `sudo pacman -Syu` — the command this feature exists for — usable at all.
It is weaker evidence and it travels labelled: `inferred: true` on the wire, "looks like it is
asking" on both boards. It fires only on a `confirm` or `numbered` shape, never on `free`, because
`free` matches any program that pauses mid-line and would put half a fleet on the board asking
nothing.

**Layer 2 never gates layer 1.** Recognising `[Y/n]` decides whether the operator gets two buttons
or a text field; it never decides whether a host appears as needing somebody. That inversion is the
difference between working on `pacman` and working on everything, and it is what the fallback rungs
are for:

- an unrecognised prompt is still answerable, as free text (`Free`);
- a command that waits having written nothing at all is still a question (`cat` does this);
- a host whose state cannot be read is **quiet**, and `blind: true` says it will never be anything
  else — a run that changes user, which is every `sudo`, lands here;
- a `secret` never renders, stores or logs the reply, and its wording is not consulted.

Two traps that produced real false positives while this was built, both now tests:

- `cat` parks in **`splice(2)`**, not `read(2)`, with fd 0 still in the first argument (#335). The
  invariant is the fd, not the syscall number.
- `pacman` redraws progress with `\r`, so every frame of the bar is unterminated text. Without
  carriage-return handling the "prompt" is every frame glued end to end.

And one race: the child parks in `read(2)` the instant it asks, which is *before* the bytes it just
wrote reach the supervisor. A question is therefore published only after one poll of agreement with
no new bytes in between — otherwise the first prompt on every run is empty.

## Privilege

A supervisor can only read a job it forked *at its own privilege*, so `sudo pacman -Syu` reports
`blind: true` — an observation made over the run rather than a sample taken once, because `spawn`
returns while the child is still this process's own un-`exec`ed copy and readable whatever it is
about to become. The node sees nothing of the process, and everything the board knows about it comes
from rung 1b: the screen.

**That is enough, and deliberately all there is.** Rung 1b covers what a privileged command actually
asks — `[Y/n]`, numbered menus, and `su`-style password prompts, all tested against a real `sudo`.
The one thing it cannot see is `sudo`'s *own* password prompt, which arrives through a relay that
hides the process and the terminal state together (#341). Every host in this herd runs passwordless
`sudo`, so that prompt does not happen here; if it ever did, the run still works and is still
answerable by opening the pane.

Closing that last case properly would mean a second binary the node starts under `sudo`, a socket
protocol for it to report on, and a new privileged surface — for one prompt that nothing in this
fleet produces. It is not built and there is no plan to build it. What was measured while working
that out stands on its own: [#334](./03-probe-log.md) for why a supervisor must share the
command's privilege at all, and [#338](./03-probe-log.md) for how such a helper would have had
to report, should the question ever come back.

## The PATH a run is given, and why it took two goes

A fleet run is forked by the node, the node is a service, and a service manager's `PATH` is not the
installing shell's — measured as `/usr/local/sbin:/usr/local/bin:/usr/bin` here
([#392](./03-probe-log.md)). So `kampr update` across the herd looked for a binary in three
directories it was never installed into. `kampr_fleet::env` reads the operator's own shell's
`PATH` once per process (`$SHELL`, then the passwd entry, then `/bin/sh`), between NUL markers so
a chatty profile's hello does not become the answer, and puts it in the child's environment rather
than running every command under a login shell — which would interleave the profile's output with
the run's and pay the profile on every host on every run.

**That fixed the rung and not the problem, on half this herd.**
[#419](./03-probe-log.md): `giftofthemagi2` and `artifactone` read their login shell correctly and
it has **no `~/.local/bin` on it**, because the profile that adds it is `.bashrc` and `-l` does not
read that. The reading is now `$SHELL -lic` first — see "The shell, and the three things it was not
allowed to break" below for what each invocation actually answers. `~/.local/bin` is where `kampr` and `herdr` are installed on all four machines. And
`kampr doctor` reported `ok`, because *a* `PATH` had been read — a check answering its own question
instead of the operator's, which is [#233](./03-probe-log.md) in miniature.

So the chosen `PATH` now has **the directory the node's own executable is in appended to it**. The
node is running from where it was installed, and `herdr` is installed beside it; that is a fact
rather than a guess, and it is *appended* so a name the chosen `PATH` already resolves goes on
resolving to the same file. This rung can add an answer and can never change one. `doctor` resolves
`kampr` and `herdr` on the final value the way `exec` does, and warns rather than saying `ok` when
it cannot find them.

`fleet.path` in `config.toml` still outranks everything, for a shell whose `PATH` a login shell
does not build — zsh puts it in `.zshrc` as often as in `.zprofile`.

## Still unmeasured

`pacman`'s replace and conflict prompts. [#336](./03-probe-log.md) could only produce the
`Proceed with installation?` one without an actual conflict to provoke the others, so their exact
wording — and whether the shapes here read them — is assumed rather than known.

## Answering several hosts at once

The commonest shape of a fleet run is every machine asking the same thing. One answer can reach all
of them, and the matching is **byte-identical, not merely similar** — the prompt, the shape, the
options and their order all have to agree, because "these two look alike" is exactly the reasoning
that sends `y` to the host that was asking something else. The hosts that are *not* being answered
are named in the confirmation: the silent third of a fleet is what bites you.

**A password is answered one host at a time.** Every password prompt in the world says
`Password:`, so a text match is no evidence at all that two hosts want the same secret, and being
wrong means handing it to the one that did not.

The Compose board is what sends; the terminal client shows the count ("· 2 more asking the same")
and answers one host at a time.

## What the node remembers, and where

A fleet run's *history* is a different thing from a cohort, and keeping them apart is the whole of
this design. `Herd::cohorts()` derives past runs from the panes that are still live: it dies on
`fleet.forget` and it dies when the node restarts, because a cohort is a set of ptys. The **fleet
book** is a record of command *strings* — five recently run, plus however many the operator kept —
and it is not a second source of truth about anything running.

**It lives on the node the client dialled, and that is structurally the hub.** The operator's phone
and their desktop are two `devices` rows in the node's own database, and there is no account above
them, so keeping the book the way `pane_prefs` is kept would have made it empty on whichever device
they picked up second — the exact opposite of the thing that was asked for. A peer, meanwhile, dials
*outbound* and has no inbound path ([ADR 0007](./adr/0007-peers-dial-outbound-to-a-hub.md)); every
device is paired against the one reachable node, and a device id minted there means nothing on a
peer. So `fleet.save` and `fleet.drop` are handled **before** a manage op is routed: a book op
carrying a `node` is still this node's book, and one relayed to a peer would have put the operator's
list on whichever machine their last pane happened to live on.

An entry is **the argv and the cwd, and nothing about which hosts it reached**. A run goes to every
host reachable when it starts; that set is different next week. The count is resolved fresh and
shown on the button.

A run is written down when the node **acknowledges** it, not when it finishes. Whether it worked is
knowable only per host and only on the host that ran it — the same privilege wall the rest of this
page is about — so a node deciding for five machines having seen one would be
[#233](./03-probe-log.md) in miniature, and it would be wrong even where it could see: answering `n`
to `sudo pacman -Syu` exits non-zero everywhere and is a perfectly good command. A history of typos
is bounded by holding five, by deduplicating, and by every entry being deletable.

A command line can carry a credential, and the automatic half of the book declines the shapes it can
recognise (`kampr_fleet::secretish`). **That is a blast-radius reduction and not a filter** — a
positional secret is invisible to it, and the blind spots are enumerated and tested in
`crates/kampr-node/tests/fixtures/secretish.json`, which the Compose client reads too so its warning
cannot drift from the node's behaviour. An explicit save is allowed, with that warning; deleting is
the part that actually holds.

And **pressing a saved command does not run it.** It fills the run box, and the ordinary "Run on N
machines" confirmation is still the only thing that fans out. One press across the whole herd should
not be cheaper than typing it.

## Using it

**On a phone or in the browser:** the fleet glyph on the herd screen opens the board; **Run** asks
for a command and sends it to every machine that can be reached, with what this node remembers under
the box — Saved, then the last five. A waiting host shows its question inline with the choices the
prompt declared, so the commonest reply is one tap.

**In the terminal client:** `prefix` then `shift+e` asks for a command and runs it everywhere;
`prefix` then `shift+f` opens the board. It does not read the book yet — the frame is additive and
a client that has never heard of it is left exactly where it was. Over the wire, see "Fleet runs" in
[`04-wire-protocol.md`](./04-wire-protocol.md).

## The shell, and the three things it was not allowed to break

**What the operator types is what runs.** `&&`, `|`, `;`, quotes, globs, `~`, redirection — a fleet
run hands the line to the host's own login shell (`$SHELL`, then the passwd entry, then `/bin/sh`)
as `<shell> -c <line>`. It used to hand an argv to `execvp`, so `;` was an argument and the sheet
carried a note telling the operator to write `sh -c '…'` themselves. The note is gone; the preview
that replaced it shows the exact line and the number of machines, which are the two things somebody
pressing Run can be surprised by.

That reversed a decision, and the decision was protecting three things. Each was measured before it
was given up.

**The shell is neither a login nor an interactive one, and the second half is the load-bearing
one.** [#337](./03-probe-log.md) says ECHO going off is an honest password signal here *because
there is no shell on the pty*, and [#333](./03-probe-log.md) says ble.sh leaves an interactive
shell's tty with ECHO already off before anything asks for anything. Measured on the operator's own
machine, with ble.sh installed and sourced from `.bashrc`: a `bash -c` fleet pty reads
`BLE_VERSION` unset and `$-` as `hBc` — no `i`, so `.bashrc` returns at its own guard — and its
termios reads **`ECHO on, ICANON on` at idle** and **`ECHO OFF, ICANON on` at a real password
prompt**, which is what a pty with nothing on it reads. `bash -i` on the same pty reads
`ECHO OFF, ICANON OFF` while merely sitting at its prompt, which is #333 reproduced. And the
mechanism goes further than the flag: ble.sh's own loader sets `_ble_init_exit=1` when
`BASH_EXECUTION_STRING` is set, so it declines to load into **any** `-c` invocation. The confound
needs a shell that is reading a line, and a fleet run never has one.
`a_password_prompt_through_the_operators_own_shell_is_still_a_secret` keeps that measured.

**A pipeline is not a question, and it looked exactly like one.** A shell brings a process tree
with it: a simple command and even an `&&` chain are `exec`ed, so the tree is one process as
before, but a pipeline is the shell in `wait4` with the members underneath it. `Procfs::waiting`
already walked descendants — it was built for `sudo`→`pacman` — so it found them. What it found was
wrong: `bash -c 'sleep 30 | cat'` parks `cat` in `splice(2)` on **fd 0**, byte for byte the line a
`cat` at a terminal produces, and fd 0 there is the pipe. Rung 1 keyed on the syscall alone puts
that host on the board as needing somebody, with nothing to answer, for as long as the pipeline
runs. Rung 1 now also requires the descriptor to resolve to **this run's own pty**
(`/proc/<pid>/fd/0`), which can only ever remove such an answer and never add one — the front of a
pipeline, which does hold the terminal, is still a question. An unreadable link is taken as the
terminal, because [#332](./03-probe-log.md) measured that an escalated job refuses `syscall` and
`fd/0` together and that combination therefore does not arise.

**And the `PATH` is read interactively, which is what [#419](./03-probe-log.md) actually needed.**
See the section above for the ladder. The reading was `$SHELL -lc`, and `-l` does not read
`.bashrc` — which is where `~/.local/bin` is added on half this herd, and where this machine adds
`~/go/bin`, `~/.nvm/…/bin` and `~/dev/houseofdoge/hod-scripts` as well. Measured from `env -i`,
which is the environment a service manager actually presents:

| invocation | answers |
|---|---|
| `bash -c` | `/usr/local/sbin:/usr/local/bin:/usr/bin` — the service manager's (#392) |
| `bash -lc` | that, plus `~/.local/bin`, flatpak, jvm, perl, rustup, Toolbox |
| `bash -ic` | `~/.local/bin`, **`~/.nvm/…/bin`, `~/go/bin`, `~/dev/houseofdoge/hod-scripts`**, the Android SDK, then the system three |
| `bash -lic` | the union of both — the only reading that is a superset of the shell the operator types into |

So `$SHELL -lic` is asked first and `$SHELL -lc` is the fallback for a shell that will not take it.
The two are asked **concurrently under one deadline**, because a fallback that only starts when the
first gives up doubles the worst case at node start. It costs 398 ms against 107 ms and is paid
**once per node process**, never per run: the value goes into the child's environment and the
per-run shell reads no profile at all. That is why the run's shell is not `-lc` — running one per
command would interleave the profile's output with the run's and pay the profile on every host on
every run.

**What the shell does not buy, and this is written down rather than left to be discovered: aliases
and shell functions.** Both live in `.bashrc`, which only an interactive shell reads, and the run's
shell deliberately is not one. The sheet says so under the box.

## What the wire carries, and what an old client sees

`fleet.run` gained an optional **`command`**, the line as typed. `args` is unchanged and is not
deprecated — an argv `exec`ed with nothing in front of it, which is what every client built before
this sends. A node given both takes `command`; a node given neither refuses with a message naming
both.

The **book** needed no new field at all. Every client renders an entry by joining `args` with
spaces, so a typed line is stored as its **single argument**: `["pacman -Syu && reboot"]` joins back
to exactly what was typed. An older client on a real phone renders it correctly and stages it
correctly; what it cannot do is run a pipeline, which it never could. Splitting the line into words
to make it look argv-shaped would have rendered `echo "hello world"` as `echo hello world` and lost
the thing that made it one argument.

Secret detection did not change and is now asserted against **both** shapes of every fixture case —
the argv, and that argv joined into one line — in Rust and in Kotlin, off the one
`crates/kampr-node/tests/fixtures/secretish.json`. The `missed` section is still asserted as missed.

**One thing moved rather than improved: what a missing binary says.** [#392](./03-probe-log.md)
gave `execvp`'s bare `No such file or directory` a sentence naming the `PATH` it had, and a shell
job never reaches that code — the shell finds the command, so a missing one is the shell's own
`command not found` on the pane and an exit code of 127. That names *which* command in a chain
failed, which the old message could not, and it does not name the `PATH`. `kampr doctor`'s fleet
path check is where that question is answered, and it still refuses to say `ok` about a `PATH` it
cannot resolve `kampr` and `herdr` on. An `args` run is unchanged and still gets the #392 sentence.

The one thing the client still refuses before a fan-out is an **unclosed quote**, because that is a
typo every host would report identically and a run nobody meant to start. A backslash escapes the
next character outside single quotes, so a command that would have worked in their own terminal is
not refused by a checker cruder than the shell it stands in front of. Everything else is the
shell's to complain about, on the host, with the host's own words.
