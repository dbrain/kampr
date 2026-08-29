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
with no shell on it, ECHO going off is honest again (#337) — the confound #333 describes needs a
shell to exist, and there is no shell here.

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

## Using it

**On a phone or in the browser:** the fleet glyph on the herd screen opens the board; **Run** asks
for a command and sends it to every machine that can be reached. A waiting host shows its question
inline with the choices the prompt declared, so the commonest reply is one tap.

**In the terminal client:** `prefix` then `shift+e` asks for a command and runs it everywhere;
`prefix` then `shift+f` opens the board. Over the wire, see "Fleet runs" in
[`04-wire-protocol.md`](./04-wire-protocol.md).

There is no shell between the operator and the command — `;` and `&&` are arguments. Ask for
`sh -c '…'` if you mean a pipeline, and see that you did.
