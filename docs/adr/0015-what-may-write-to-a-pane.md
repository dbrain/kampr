# 0015 — What may write to a pane, and why it is never a side effect of looking

- **Status:** Accepted
- **Date:** 2026-10-14
- **Records:** what `AGENTS.md` rule 3 said in full before it was cut to a summary. Nothing here is
  a new decision; it is the part of the rule that no other ADR wrote down.
- **Evidence:** probes [#17, #18, #19, #298, #357, #515](../03-probe-log.md)
- **Depends on:** [ADR 0012](./0012-one-deliberate-resize-behind-a-panel.md),
  [ADR 0013](./0013-a-standing-intent-to-match-the-view.md)

## Context

ADRs 0002, 0012 and 0013 decide *when the geometry* of a pane may change. Three things the
project also holds to were written only in the root `AGENTS.md`, where every agent paid for them on
every session and none could find them by topic: that focus is a write, that a pane Kampr spawned
for itself is governed differently from one it found, and that the same rule covers writes that
are not geometry at all.

The rule underneath all of them is one sentence: **nothing reshapes or marks a pane because
somebody looked at it.** It turns on *whose* pane it is and *why* the write happens. It is not a
ban on any op.

## Decision

**A pane Kampr found belongs to the operator.** It may have a desk attached on the machine itself,
and `control` takes the PTY with no way to decline ([#17](../03-probe-log.md)), overrides that desk
while held ([#18](../03-probe-log.md)) and leaves it rendering wrong without being told
([#298](../03-probe-log.md)). No watch, zoom, pan, fit, reconnect or layout may claim one. The
paths that do take its PTY, each one asked for by a person and each released, are:

1. the panel's `pane.size`, which resizes once and lets go (ADR 0012);
2. the standing intent to match a desk-sized view, owned by the socket that claimed it and put
   back on release, through the same op (ADR 0013) — and the restore that puts it back;
3. `kampr resize`, the operator at the machine's own shell, for a pane an agent spun up headless
   at whatever size that shell was. It claims, releases on a timer, and then *measures* whether
   the size stuck rather than printing that it did ([#19](../03-probe-log.md)).

`crates/kampr-testkit/tests/guards/claim_sites.rs` fails on a fourth caller of
`Controller::claim`.

**Kampr never focuses a pane on its own account.** Focus is not a read: it destroys herdr's `done`
marker, the state herdr synthesises for a pane that finished `working`→`idle` while unfocused and
which is the operator's unread flag ([#357](../03-probe-log.md)). `pane.focus`, `tab.focus` and
`workspace.focus` all clear it, because herdr's rule is "whatever makes the pane the
session-focused pane", and Kampr's `focus` manage op routes all three. **`pane.zoom` clears it
too**, for the whole tab and even when the zoom is a no-op, because herdr routes it through focus
before deciding whether anything changed ([#515](../03-probe-log.md)). Every read leaves it
standing. So focus is a thing the operator presses, told what it costs; every create op passes
`focus: false`; and a new op is measured against `done` before it is used, not assumed safe.

**A pane Kampr created for a job of its own is Kampr's** — a short-lived TTY it spawned, ran a
command in and will tear down. It may be given a geometry at creation and while Kampr holds it,
because there is no operator desk to trample and no other viewer's geometry to lose. Ownership is
a property of the pane and it ends: the moment the operator adopts one, it is a found pane.

**The same distinction governs writes that are not geometry.** `convo.composer`'s takeover empties
the pane's own composer — behind a control, with the words moved into the reply box first and the
keystroke measured per harness. That is the operator pressing a thing. What the rule forbids is
the write that happens on a view switch, a reconnect, a fit or a layout.

## Consequences

- A phone narrower than a pane renders it — zoom, pan, the conversation view — and never takes the
  pane down to its own size.
- A client cannot offer "jump to this pane at the desk" as a convenience; it is a pressed action
  with its cost stated.
- Every new herdr op has a measurement owed before it is wired: does it move geometry, and does it
  clear `done`.

## What would justify revisiting

herdr offering a way to observe or address a pane that neither takes its PTY nor passes through
focus — a declinable `control`, or a zoom that does not route through focus. Either would remove
the cost these rules exist to avoid, and the rules should shrink with it.
