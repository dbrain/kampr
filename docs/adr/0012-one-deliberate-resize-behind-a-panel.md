# 0012 — One deliberate resize, behind a panel

- **Status:** Accepted
- **Date:** 2026-08-28
- **Supersedes:** [ADR 0002](./0002-kampr-never-resizes-a-pane.md), in part — the invariant it
  states is narrowed, not abandoned
- **Evidence:** probes [#14, #17–#21, #219, #221, #265, #298](../03-probe-log.md), and the two rows
  measured for this decision

## Context

ADR 0002 said Kampr never resizes a pane, that this was **structural rather than policy** — "the
code path does not exist" — and it listed what would justify revisiting. Neither trigger has fired:
herdr has not shipped per-client geometry, and it has not shipped a read-only `control`. The ADR
also pre-emptively refused the third reason, in as many words:

> **Nothing about a better phone UI.** Zoom being awkward, a wrapped line being ugly, a user asking
> for it — none of these reopen the question.

**This ADR is that third reason, and it is worth being honest that it is.** What changed is not the
cost of a controller, which is exactly what 0002 measured. What changed is that a case turned up
which no amount of rendering can reach, and 0002 had not seen it.

### The case

An agent starts a herdr headlessly. The pane is born at whatever size that shell was — often far
narrower than anything usable — and it stays there for ever. Every instrument Kampr had was measured
against this and none of them touch it:

| | |
|---|---|
| Zoom, pan, the conversation view | Rendering. A 40-column pane really is 40 columns; magnifying it magnifies the crop. |
| `observe` | Never touches the PTY ([#14](../03-probe-log.md)), and observers at different sizes coexist ([#13](../03-probe-log.md)). |
| `pane.zoom`, which Kampr already ships | Moves the PTY **only with a client attached** and does nothing at all headless ([#265](../03-probe-log.md)). |
| Anything on the socket API | Nothing reports or sets a column count, across all 91 methods ([#221](../03-probe-log.md)). |
| `stty` inside the pane | Moves the kernel winsize only; herdr goes on wrapping at its own grid width ([#221](../03-probe-log.md)). |

A controller is not *a* way to reach it. It is the only one.

### What a controller still costs

Everything 0002 said, and one thing it did not live to see:

- `control` **always** claims the PTY, with no flag to decline ([#17](../03-probe-log.md)).
- It overrides the desk while held ([#18](../03-probe-log.md)).
- A *frozen* controller holds the PTY for ever; herdr never reclaims it ([#20](../03-probe-log.md)).
- Against an attached desk TUI it **neither refuses nor evicts** — the desk simply renders wrong and
  is told nothing ([#298](../03-probe-log.md)). This is 0002's invariant with a face on it, and it
  is why the scope below is what it is.

And the asymmetry that makes a narrow exception viable at all:

- Headless, the size **persists** after the controller goes ([#219](../03-probe-log.md), and the
  enlarging case measured for this ADR).
- Attached, release restores the desk's own geometry within a second
  ([#19](../03-probe-log.md)).

So a controller is *useful* in exactly the case `pane.zoom` cannot serve, and *useless* in exactly
the case `pane.zoom` already serves. They are complements, not alternatives.

## Decision

**Kampr resizes a pane in one place, only when a person asks for it by name, and never as a side
effect of viewing.** ADR 0002's real invariant — *looking at a pane changes nothing about it* — is
untouched and remains absolute. What is narrowed is the claim that the code path does not exist.

The op is `pane.size`. Its shape is the decision:

1. **It is never implicit.** No watch, no zoom, no pan, no fit, no reconnect and no layout reaches
   it. It is a panel an operator opens and a button they press, and in the terminal client a
   confirmation that names the consequence before anything is claimed.

2. **It always releases.** The default mode claims, resizes, hands the PTY back, and the
   controller's life is measured in hundreds of milliseconds. A release that does not land inside
   three seconds becomes a kill — that is the whole answer to [#20](../03-probe-log.md), and it is
   the thing `docs/01-implementation-findings.md` named as Kampr's to build.

3. **Holding is opt-in, and bounded.** A hold is what makes a size survive on an attached pane, and
   it costs that desk a wrong-looking screen ([#298](../03-probe-log.md)). It is a toggle that
   defaults to off, is session-local rather than remembered, states the consequence when ticked,
   and is released by a deadline whatever the client does.

4. **It refuses to make a pane unusable.** 80x24 is a floor, not a suggestion. Because a headless
   resize persists, a client that fitted a pane to its own viewport would lock that pane at phone
   width for every other client, with nothing but another resize to undo it. The escape hatch must
   not become the thing it exists to escape.

5. **It measures, and says what it found.** Rows are read back from `scroll.viewport_rows`, which is
   the PTY's and not the rect's ([#84](../03-probe-log.md)); columns are reported by nothing
   anywhere ([#221](../03-probe-log.md)). On an attached pane the size reverts, so a reply that
   echoed the request would be a plausible-looking success — the failure class that cost this
   project the most ([#233](../03-probe-log.md)).

**Still refused, and not by policy:** `terminal.scroll`. It rides the same channel, and ADR 0004
priced scrollback on its absence. Gaining a controller for a deliberate resize does not make it a
scroll transport, and nothing here reopens that.

## Consequences

- **`README.md`'s "structurally incapable" is no longer true and has been rewritten.** So has
  `ARCHITECTURE.md`'s "*cannot*, not does not". A claim that is no longer true is worse than a
  weaker claim that is.
- `ADR 0004` is unaffected in substance: it refuses `terminal.scroll`, which is still refused.
- `ADR 0011`'s "`terminal.resize` appears nowhere in this crate" remains true of `kampr-tui` — the
  controller lives in `kampr-herdr` and is driven by the node.
- The wire stays additive: a new `op` value and two new optional fields. An older node answers
  `unsupported`; an older client never sends it.
- `docs/11-cli-briefs.md` W7 is what this implements, minus its headless-only scope — the hold mode
  exists precisely for the attached case, with the consequence stated rather than avoided.

## What would justify revisiting

- **Herdr shipping per-client geometry** (upstream U3). That removes the trade entirely and would
  let this op be deleted rather than merely narrowed. Still the outcome to pursue.
- **A controller that can decline geometry** (a read-only `control`). It would make the hold mode
  harmless and would let ADR 0004 be revisited at the same time.
- **Evidence that the panel is being reached by accident.** The whole safety of this rests on it
  being deliberate. If a resize ever happens that an operator did not intend, that is not a bug to
  fix in the panel — it is this decision failing, and it should be reversed.
