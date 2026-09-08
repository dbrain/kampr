# 0014 — The pane's width is read, not inferred

- **Status:** Accepted
- **Date:** 2026-09-08
- **Shipped in:** _(set at the release commit)_
- **Evidence:** probes [#68](../03-probe-log.md), [#84](../03-probe-log.md),
  [#211](../03-probe-log.md), [#218](../03-probe-log.md), [#220](../03-probe-log.md),
  [#221](../03-probe-log.md), [#229](../03-probe-log.md), [#230](../03-probe-log.md),
  [#509](../03-probe-log.md), [#510](../03-probe-log.md), [#513](../03-probe-log.md),
  [#516](../03-probe-log.md)
- **Depends on:** [ADR 0003](./0003-the-client-contract-is-a-cell-grid.md)

## Context

A stream has to be opened at the pane's real width or every row is wrong: `observe --cols` **crops
and does not reflow** ([#15](../03-probe-log.md)), and observing above the PTY pads while the
`width` a frame reports merely echoes the request ([#87](../03-probe-log.md)). The layout rect is
not that number — headless, a pane whose rect said 47 had a 93-column PTY ([#68](../03-probe-log.md))
— and [#221](../03-probe-log.md) established that **nothing in herdr's socket API reports a column
count at all**.

So the node inferred it, from the only surface that renders at the true width: two `pane.read`s,
walked from the bottom, pairing each logical line with the rows that rebuild it
([#84](../03-probe-log.md), [#217](../03-probe-log.md)). It worked, and it cost about four hundred
lines carrying a running floor, a proof that decayed after twenty unconfirmed readings because
nothing announces a PTY resize ([#211](../03-probe-log.md)), a per-row walk because the rows of a
join do not share a stride ([#229](../03-probe-log.md)), and a `commanded` override because the
rows already on screen were laid out at the width *before* a resize — which on the operator's own
hub streamed a 289-column pane at 292 until those rows scrolled away. It was rewritten twice, and
it still could not separate `n` from `n + 1` on a screen of nothing but wide glyphs, which reads
back byte for byte the same on both grids ([#220](../03-probe-log.md)).

**herdr 0.9 answers the question by accident.** `pane.selection.read` refuses a cursor column at or
past the grid width with `selection_unavailable`, on any valid row and whatever that row holds — a
blank row and a row of `中` bound identically ([#509](../03-probe-log.md)). So the width is the one
`W` with `fits(W - 1)` and `!fits(W)`: about twelve pure reads cold, **two** to confirm a candidate,
1.5 ms and 0.18 ms locally. It agreed with a `control`-driven resize after 3 ms and 5 ms at two
sizes. It moves no scroll offset and does not clear `done` ([#515](../03-probe-log.md)).

[#221](../03-probe-log.md) still stands as written, re-checked across every method and event of
protocol 22: nothing *reports* a column count. What is new is a method that *bounds* on one.

## Decision

**Read the width. Delete the inference — all of it, with no fallback.**

`Herdr::pane_width` is the one source, seeded with the width last read or, on the first pass, the
layout rect, which is the width or one more than it — the column herdr keeps back for the scrollbar
([#230](../03-probe-log.md)) — and so is right half the time for nothing. Its only caller in the
sweep is `HerdrProvider::observe_cols`.

A read that does not answer **leaves the last width standing** rather than falling back to the
rect. herdr being briefly unreachable is not evidence that a pane got narrower.

**herdr 0.9.0 becomes a hard floor**, enforced in `kampr doctor` and declared in
`herdr-plugin.toml`.

## Consequences

- Four hundred lines go, and with them the `#220` ambiguity, which is closed rather than narrowed:
  the bound is on the grid, not on the content.
- `commanded` goes too. It existed only because an inference could overwrite a width just
  commanded, and a read that bounds on the live grid cannot.
- **`measured_cols` gets better, not just simpler.** It is the width a hold restores to, and it
  used to be `None` until a wrap had proved one — so the full-screen agent whose pane a hold most
  wanted to restore was the pane there was nothing to restore *to*. It now has an answer from the
  first sweep. Still `None` before that: putting the rect back would be a resize to a number no row
  was ever laid out at.
- **One rule-3 hazard is deleted for free.** The inference's `pane.read` used `format: "text"` with
  `source: "recent"`/`"recent_unwrapped"`, which on an idle alt-screen agent pane makes herdr inject
  synthetic mouse-wheel events into the operator's TUI to harvest history — 5.3 s, measured, and
  present on 0.8.2 as well ([#513](../03-probe-log.md), superseding
  [#231](../03-probe-log.md)). That call site is gone; `read_scrollback` stays on `format: "ansi"`
  and a test named for the defect pins it there.
- The steady-state cost is two socket round trips per pane per width read, against two `pane.read`s
  before. A cold read is twelve, and only a pane that has moved pays for it.
- **A herdr below 0.9.0 will not work at all**, rather than working worse. That is the point of a
  floor, and it is not the whole risk: `terminal session observe` is version-locked in both
  directions while the JSON socket is not ([#516](../03-probe-log.md)), so a half-upgraded install
  answers every question correctly and streams nothing — [#233](../03-probe-log.md) exactly. The
  binary and the server move together.

## What would justify revisiting

- **herdr reporting a column count outright.** One field on `pane.get` and this becomes one read
  instead of two, and the binary search goes. The bound is a happy accident of a selection API; a
  reported number would be a contract.
- **The bound turning out to be narrower than the grid** on some pane this has not met — a pane
  with zero rows, or a width that moves without a controller, both of which
  [#509](../03-probe-log.md) flags as unmeasured. The symptom would be a stream one column short,
  which crops; the guard is that the cold search and the ground truth agreed at every width tested.
- **A structured-grid socket**, which would supersede far more than this — see
  [ADR 0001](./0001-the-node-runs-a-vt-emulator.md), which names it as its own supersession
  condition.
