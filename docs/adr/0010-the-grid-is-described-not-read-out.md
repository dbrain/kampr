# 0010 — The terminal grid is described, and only the cursor line is spoken

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** _(set at the release commit)_
- **Evidence:** probes [#92–#94](../03-probe-log.md) — a TalkBack session against a live 94×40
  herdr pane; `GridAccessibilityTest` holds every rule below and fails the build if one moves
- **Depends on:** [ADR 0003](./0003-the-client-contract-is-a-cell-grid.md),
  [ADR 0005](./0005-structure-comes-from-the-transcript.md)

## Context

[ADR 0003](./0003-the-client-contract-is-a-cell-grid.md) settles that the client is handed a cell
grid. Everything else in the client is composed text and takes a `contentDescription` without
argument. The grid does not, and it is the one surface where the obvious fix is the wrong one.

**A grid is spatial, and speech is linear.** The pane probed for this ADR was 94 columns by 40
rows, and became 206 columns wide when the desk resized it. Reading the viewport aloud is 3,760
characters at the first width and 8,240 at the second. TalkBack's default rate is roughly 200
words per minute — call it 16 characters a second — so one repaint is **four minutes of speech at
94 columns and nine at 206**. That is before asking what it would say.

**It would say the wrong thing.** `PaneState.revision` increments on every `grid.patch`, and probe
#56 has the renderer holding 60 fps. A live region bound to grid contents restarts its utterance
sixty times a second and never finishes a sentence. And a terminal's columns carry meaning that
row-major reading destroys: `htop`, a side-by-side diff, `git log --graph`, any two-column TUI —
read left to right, row by row, they interleave into nonsense. Kampr already has the code that
proves the distinction: `LogicalText.lineAt` joins soft-wrapped rows back into the line the
program actually wrote, which is exactly why *one* line is worth speaking and forty are not.

**What a blind operator uses a terminal with today does not read the viewport either.** Orca with
gnome-terminal, NVDA with Windows Terminal, and BRLTTY on a console all do the same two things:
they echo the line the cursor is on as it changes, and they put viewport review behind explicit,
separate commands with their own cursor. The screen reader is a client of the terminal's cursor,
and review is a mode you enter, not a thing that is read at you. Kampr shipping "the whole screen,
continuously" would not be more accessible than those tools; it would be less.

**Kampr has a second surface none of those tools have.** By
[ADR 0005](./0005-structure-comes-from-the-transcript.md) an agent pane's structure comes from the
transcript, and the conversation view renders it as ordinary composed text — turns with roles,
code blocks, tool calls, a reply box, and a pending-answer strip that can answer a blocked prompt
without touching the grid at all. `AppState.openPane` already prefers it for an agent pane with no
ring. For the panes Kampr exists to look after, **the accessible surface already shipped, and the
grid is the fallback.** Saying so out loud is part of this decision rather than an excuse for it.

## Decision

**The grid describes itself; it never reads itself out. The line under the cursor is the unit of
speech, and where a transcript exists the grid points at it.**

Four rules:

1. **The grid is one semantics node carrying a description of the surface, not of its contents** —
   size, cursor position, depth of history, stale, read-only. Its click action is labelled *Type
   into this pane* and raises the keyboard, because a terminal you cannot type into is a
   screenshot.

2. **The cursor line is spoken, on a polite live region, coalesced.** A separate one-dp node
   carries `LogicalText.lineAt(cursor)` and updates after **450 ms of quiet** on `revision`. Polite
   rather than assertive: the pane talking is not an interruption, the operator asking a question
   and getting an answer is. The settle window is what makes a live region legal on a surface that
   repaints at frame rate.

3. **A pane with a transcript says so in its own description, and offers a custom action to go
   there.** Not a hint in a tooltip — a named action a screen reader can list and invoke, so the
   route to the readable surface does not require finding a tab by touch.

4. **The grid is never a text field.** No `editableText`, no `textSelectionRange`, no fabricated
   linearisation with offsets. Offsets into a buffer that is rewritten under the reader's cursor
   are worse than no offsets.

## Consequences

**What this buys.** With TalkBack on and a real pane, an operator can now hear what the pane is,
where the cursor sits, how much history is behind it, whether it has gone stale, and what the
prompt currently says — and can type, because the key row's caps carry semantics actions and the
grid raises the keyboard. In the session behind probe #92 the caps went from *unnamed and
unpressable* to a double-tap that put characters in the shell.

**What it does not solve, and this list is the point:**

- **There is no review mode.** A screen reader user cannot walk the viewport, cannot ask for row
  7, cannot read a column, cannot re-read the line above. The cursor line is all there is.
- **A full-screen TUI is opaque.** `htop`, `vim`, `less`, an ncurses installer — their content is
  not on the cursor line, so Kampr conveys essentially nothing about them. This is the largest
  hole and it is not closed by anything short of rule 4's opposite plus a navigation model.
- **No braille, and no cursor routing.** Compose Multiplatform exposes no braille surface, and
  nothing lets a reader route the terminal cursor by naming a cell.
- **Selection is sighted-only.** The handles are dragged at pixel positions; *Copy the selection*
  is named, but nothing can set the range without sight.
- **Speech lags and drops.** A fast build log speaks the last settled line, not every line. That
  is deliberate — the alternative is speech that never completes — and it is still a loss.
- **The reasoning is from how console screen readers behave, not from blind terminal operators
  using Kampr.** No such user has tried it. That is the weakest joint in this ADR and the first
  thing that should overturn it.

**What it costs to run.** One `LogicalText.lineAt` per 450 ms of quiet rather than per frame. The
walk backwards over soft-wrapped rows is bounded by the wrap chain, not by the ring.

## What would justify revisiting

- **A real review mode, designed as one.** A reader-owned cursor independent of the pane's, with
  read-row, read-column and read-to-end, and a defined answer for what happens when a `grid.patch`
  lands underneath it. That is a feature with a navigation model, not a `contentDescription`, and
  it is the honest missing piece.
- **A blind operator actually using this.** One session would outrank every paragraph above.
- **Braille support in Compose Multiplatform.** A braille display is the one device that makes a
  cell grid genuinely legible, because it is spatial too.
- **A signal marking part of the grid as a document.** If a harness or herdr could say "these rows
  are prose", rule 1 could exempt that region and read it.

## Alternatives closed off

- **Expose the visible rows as the grid's `contentDescription`.** The measurement above: minutes
  of speech per repaint, restarted at frame rate, and spatially wrong for anything columnar. Its
  real defect is that it *looks* like the problem is solved, which is worse than an honest gap.
- **Make the grid a read-only text field** so a screen reader's own line, word and character
  navigation supplies the review mode. This is the tempting one, and rule 4 exists to refuse it:
  the text would be a linearisation Kampr invented, with offsets that every patch invalidates
  under the reader's cursor. A review mode has to own its cursor; borrowing the text framework's
  cursor over a buffer that moves is a worse experience than none.
- **A "read the screen" button** emitting one long utterance on demand. The wall of text with an
  extra tap, and it disguises the fact that review is unimplemented.
- **Announcing every changed row as it changes.** The same 60 Hz firehose as reading the viewport,
  with the added defect that it speaks output the operator did not ask for while they are reading
  something else.
