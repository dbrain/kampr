# Kampr

Remote access to your herd: a Rust node beside [herdr](https://herdr.dev/) on each machine, and a
Compose Multiplatform client that reaches it from a phone or a browser.

This file is the front door, for any agent or any person. The documentation behind it is thorough
and worth trusting — the point of this page is to tell you which part to read for the question you
have, and to name the handful of rules that are not negotiable. `CLAUDE.md` is a symlink to this
file; there is one brief, not two.

**If you are here to review rather than to build, read "Reviewing this codebase" at the bottom
first.** Several deliberate conventions here look like defects if you do not know they are
deliberate, and the fastest way to be useless is to file thirty findings about missing docstrings.

## Read this much before you change anything

| Question | Read |
|---|---|
| What is this and how do I run it? | [`README.md`](./README.md) |
| How does it work, and why that way? | [`ARCHITECTURE.md`](./ARCHITECTURE.md), then [`docs/adr/`](./docs/adr/) |
| What does herdr actually do? | [`docs/03-probe-log.md`](./docs/03-probe-log.md) — **read this before assuming anything** |
| What goes over the wire? | [`docs/04-wire-protocol.md`](./docs/04-wire-protocol.md) |
| Why is my build broken? | [`docs/09-toolchain.md`](./docs/09-toolchain.md) |
| How do I ship it? | [`docs/07-android-release.md`](./docs/07-android-release.md) |
| Why won't passkeys work on my host? | [`docs/10-passkeys.md`](./docs/10-passkeys.md) — the Android asset-links constraint is structural |

## The rules

1. **The probe log is the source of truth about herdr.** If you need a fact about herdr that is not
   in [`docs/03-probe-log.md`](./docs/03-probe-log.md), *probe it and add a row*. Do not assume, do
   not reason from another terminal multiplexer, and do not trust a memory. Numbers are permanent
   identifiers that code and docs cite: **append, never renumber**, and supersede by striking through
   with a cross-reference. If two tasks are appending at once, propose rows unnumbered and let one
   writer assign.

2. **Measure, do not reason.** Nearly every defect in this project's history had a mechanism unlike
   its symptom. Reproduce it in a harness first; then prove the fix is load-bearing by reverting it
   and watching the test fail. A test that still passes with the defect restored is a harness that
   was never the app (#191) — delete it rather than keep it green.

3. **Kampr never resizes a pane.** No `terminal session control`, no `terminal.resize`, ever
   (#17, #18). Small screens are handled by rendering: zoom, pan, and the conversation view.

4. **Bug fixes and logic changes are TDD**, at integration level where one exists. `live.rs` drives a
   real herdr end to end and is usually the honest level.

5. **Zero comments by default.** Names and types are the documentation. Comment only a non-obvious
   *why* — a constraint, a workaround, an invariant a reader cannot guess. The existing crates show
   the intended density; match it rather than adding docstrings that restate the code.

## Traps that have cost real time

- **`env -u GRADLE_HOME ./gradlew`**, always, from `client/`. A `GRADLE_HOME` on this machine
  silently overrides the wrapper (#67). The `Makefile` does this for you.
- **A node reaches herdr two ways** — a socket, and a spawned `herdr terminal session observe`. They
  fail independently, and every surface a person can see is served by the socket, so a node whose
  binary half is broken looks completely healthy with every pane blank (#233). `kampr doctor`'s
  `observe` check answers this directly.
- **Live tests need a real herdr on PATH** and are sensitive to machine load and to stray sessions —
  a node serves *every* herdr session it can find (#97). Use a throwaway named session and tear it
  down. Re-run a failure alone before calling it a regression.
- **The layout rect is not the PTY.** Neither its width (#68, #84) nor its height (#205) — it is the
  pane's outer box, and the column it keeps back is the scrollbar's (#230). `viewport_rows` is
  honest; the width has to be inferred and that machinery is `ARCHITECTURE.md` §4.2.
- **The wire is additive only.** Older clients are installed on real phones. Unknown `t` values and
  unknown fields are ignored by rule; an unrecognised `error.code` must still render its `message`.

## Where the code lives

| Path | What |
|---|---|
| `crates/kampr-herdr` | The socket RPC, the snapshot model, the `observe` supervisor, binary resolution |
| `crates/kampr-term` | The `vte` emulator: bytes → cell grid, clustering, dirty rows |
| `crates/kampr-core` | Providers, pane registry, stream supervision, width inference, the wire encoder |
| `crates/kampr-node` | HTTP/websocket server, herd model, sessions, manage ops |
| `crates/kampr-auth`, `-mesh`, `-push`, `-journal`, `-cli`, `-spike` | Pairing and tokens; hub/peer links; web push; agent transcripts; the `kampr` binary; the fidelity canary |
| `client/shared` | Model, wire, theme, and the screens that are not a pane |
| `client/terminal`, `client/conversation`, `client/mosaic` | The three pane surfaces |
| `client/androidApp`, `client/webApp`, `client/desktopApp` | The composition roots. Android and web are the two that ship |

`docs/05-agent-briefs.md` is the original build plan and is kept for its reasoning; treat its
"what already exists" table as history, not as the current state.

## Reviewing this codebase

Written for a reviewer arriving cold, human or otherwise. Several conventions here are deliberate
and read as defects if you do not know that.

### Do not flag these — they are the house style

- **Absent docstrings and absent comments.** Rule 5 is real: names and types are the documentation,
  and a comment that restates its code is a defect here, not a virtue. A function with no doc
  comment is finished, not undocumented.
- **`#123` inside a comment.** That is a citation into
  [`docs/03-probe-log.md`](./docs/03-probe-log.md), not a ticket and not a TODO. Every row there is
  a measurement with the command that produced it. If a comment cites one, the claim above it was
  measured — go and read the row before disagreeing with it.
- **Constants that look arbitrary.** Most trace to a measurement: a 40-row grid, a 20-reading proof
  lifetime, a 4-cell margin, a 22 dp strip. Search the probe log before calling one magic.
- **Long prose comments explaining *why*.** They exist where a reader could not otherwise guess a
  constraint, and they are usually the only record of an expensive lesson.
- **Prose commit messages and test names that read as sentences.** Deliberate. A test named
  `a_node_that_cannot_run_herdr_says_so_instead_of_promising_a_grid` is naming the defect it guards.
- **Allocation-free paths in the terminal renderer and emulator.** Text shaping is the entire cost
  of a frame (#58–#62) and run-layout caching is required, not an optimisation. A refactor that adds
  an allocation per cell or per frame is a regression however much cleaner it reads.

### Worth flagging, and this project will thank you

- **A wire change that is not additive.** Old clients are on real phones. New optional fields and
  new `t` values are fine; reinterpreting an existing field is not.
- **Anything that resizes a pane** — see rule 3. It is the one thing Kampr must never do.
- **A claim about herdr's behaviour with no probe row behind it**, in code or in a comment.
- **A citation pointing at a probe number that does not exist.** This has happened; the log is
  append-only and gap-free, so `grep -c` the number before trusting it.
- **A test that would still pass with the defect restored.** The strongest review finding available
  here: name the mutation and say what should have failed.
- **Dead code** — defined but never called outside its own tests. A component that exists and is not
  wired to anything is not finished.
- **A `catch`/`unwrap_or` that turns a failure into a plausible-looking success.** The most
  expensive bug in this project's history was a node that answered every question correctly while
  one of its two paths to herdr was dead (#233).

### Running it yourself

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd client && env -u GRADLE_HOME ./gradlew check
```

All four must be clean. The Rust suite includes live tests that drive a **real** `herdr` binary; if
one is missing they skip loudly rather than pass quietly, and they are sensitive to machine load —
re-run a failure alone before reporting it.
