# Kampr

Remote access to your herd: a Rust node beside [herdr](https://herdr.dev/) on each machine, and a
Compose Multiplatform client that reaches it from a phone or a browser.

`CLAUDE.md` is a symlink to this file. `client/` and the crates with traps of their own (`kampr-core`,
`-node`, `-herdr`, `-fleet`, `-journal`) carry an `AGENTS.md` beside their code. **Reviewing rather
than building? Read [`.claude/skills/kampr-review/SKILL.md`](./.claude/skills/kampr-review/SKILL.md)
first.** Several deliberate conventions here look like defects without it.

## Check your work

- `make test-fast` runs the ~1,500 tests that need no herdr, in about 20 s. Use it as the inner loop.
- `make check` is the gate: fmt, clippy, every test including the live herdr suites, the client
  and packaging. CI runs the same targets, so green here means green there.

## Where to look

| Question | Read |
|---|---|
| How does it work, and why that way? | [`ARCHITECTURE.md`](./ARCHITECTURE.md), [`docs/adr/`](./docs/adr/) |
| What does herdr actually do? | `scripts/probe <words or #>`. The [probe log](./docs/03-probe-log.md) is 600 KB; search it, never read it whole |
| Wire protocol / threat model | [`docs/04`](./docs/04-wire-protocol.md) / [`docs/08`](./docs/08-threat-model.md) |
| Build broken? Shipping? | [`docs/09-toolchain.md`](./docs/09-toolchain.md) / [`docs/07-android-release.md`](./docs/07-android-release.md) |
| Mesh, notifications, passkeys | [`docs/15`](./docs/15-mesh-deployment.md), [`docs/16`](./docs/16-notifications.md), [`docs/10`](./docs/10-passkeys.md) |
| Terminal client, harness transcripts, fleet runs, agent status | [`docs/11`](./docs/11-cli-briefs.md), [`docs/12`](./docs/12-harness-briefs.md), [`docs/13`](./docs/13-fleet-runs.md), [`docs/14`](./docs/14-agent-status.md) |
| Native sessions, muffit sessions | [`docs/17`](./docs/17-native-sessions.md), [`docs/18`](./docs/18-muffit.md) |

`docs/01`, `02`, `05` and `06` are history, not status.

## The rules

1. **The probe log is the only source of facts about herdr.** If a fact isn't there, probe it and
   append a row ([`probe-herdr` skill](./.claude/skills/probe-herdr/SKILL.md)). Never renumber a
   row. A guard test enforces this.
2. **Measure, don't reason.** Reproduce in a harness first, then revert the fix and watch the test
   fail. If a test still passes with the defect restored, delete it (#191).
3. **Nothing reshapes or focuses a pane because somebody looked at it.** Only a person's explicit
   request may take a found pane's PTY. [ADR 0015](./docs/adr/0015-what-may-write-to-a-pane.md) is
   the whole rule, and a guard test fails on a new `Controller::claim` caller.
4. **Bug fixes and logic changes are TDD**, at integration level where one exists
   (`crates/kampr-node/tests/it/live.rs` drives a real herdr).
5. **Zero comments by default.** Comment only a non-obvious *why*: a constraint, a workaround, an
   invariant or a probe citation. Don't add docstrings that restate the code.
