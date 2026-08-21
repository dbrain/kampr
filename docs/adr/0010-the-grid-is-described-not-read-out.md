# 0010 — The terminal grid is described, and only the cursor line is spoken

- **Status:** Accepted. Amended 2026-08-21 with the review mode this ADR had named as its own
  largest hole
- **Date:** 2026-08-20, amended 2026-08-21
- **Shipped in:** _(set at the release commit)_
- **Evidence:** probes [#92–#94](../03-probe-log.md) — a TalkBack session against a live 94×40
  herdr pane. The review mode of rule 5 and the history marks of rule 1 were driven under TalkBack
  on an API 37 emulator against a live `kampr serve` against a real herdr session — probes
  [#108–#111](../03-probe-log.md), of which [#110](../03-probe-log.md) is the one to read before
  changing anything: a Compose live region will not re-speak identical text, and *read this row
  again* depends on a trick to defeat that. `GridAccessibilityTest`, `GridReviewTest`,
  `ScrollbackHonestyTest`,
  `ReviewTest` and `LiveScrollbackTest` hold every rule below and fail the build if one moves
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

### What the review mode had to solve, and it is not the cursor

The first version of this ADR listed "there is no review mode" first among the things it did not
solve, and named the shape of the answer: a reader-owned cursor with a defined answer for what
happens when a `grid.patch` lands underneath it. Writing that cursor is an afternoon. **The
defined answer is the whole problem**, and it has two halves that pull in opposite directions.

**A reader parked on a row must not be moved.** The surface a reader walks is history and the live
grid addressed by one index (`SurfaceRows`), and that index is not stable: history arriving pushes
the live viewport down it, and a `scrollback` discard advances `from_top` and shortens it from the
front. A cursor held as a raw surface index is silently pointed at a different line by a message
the reader never asked for. So the two halves of the surface are anchored differently — history by
its absolute ring index, which survives more history arriving; the grid by its own viewport row,
which is what "the third line from the bottom of the screen" means.

**And a reader parked on a row must not be lied to.** The live half of that surface is repainted
under them by definition. Anchoring to the row is right — it is the screen position they chose —
but the text at that position changes, and a review mode that reads them stale content it captured
minutes ago is worse than one that moves. The only honest options are to speak the change or to
mark it, and speaking it is the 60 Hz firehose this ADR exists to refuse.

## Decision

**The grid describes itself; it never reads itself out. The line under the cursor is the unit of
speech, and where a transcript exists the grid points at it. Walking the grid is a mode the reader
enters, owns the cursor of, and is never moved inside.**

Five rules:

1. **The grid is one semantics node carrying a description of the surface, not of its contents** —
   size, cursor position, depth of history, **whether that history is whole**, stale, read-only.
   Its click action is labelled *Type into this pane* and raises the keyboard, because a terminal
   you cannot type into is a screenshot. The history clause is present only when the record is
   broken: `capped` and `complete` are two different losses and it says which, in rows, and a ring
   with no hole in it says nothing at all. The same fact is written at the top of the scrollable
   surface, where it is reached by scrolling to it rather than worn as a badge.

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

5. **Review is a mode, its cursor belongs to the reader, and while it is on the pane stops
   talking.** Four parts, and each of them is a decision rather than a detail:

   - **It is entered and left deliberately** — a named custom action on the grid, a *review* pill
     beside the column indicator, and a strip of real buttons while it is on: previous and next
     row, previous and next word, read this row again, back to the live cursor, leave. Real
     buttons, not gestures: TalkBack's double tap and a keyboard's Return both dispatch a semantic
     click and neither reaches a `pointerInput` block. With focus in the strip the arrow keys move
     the cursor and Escape leaves, so the mode is drivable without ever finding a button.
   - **The unit is the grid row, not the logical line.** Rule 2 speaks the unwrapped line because
     that is what the program wrote; review speaks the row because that is what is on the screen,
     and `htop` and `vim` — the two panes this mode exists for — have nothing to say about which
     rows were once one line.
   - **A repaint underneath the cursor never moves it and never speaks.** The anchor resolves
     against the surface as it is now, so history growing slides the index without moving the
     reader and a discard is noticed rather than absorbed. When the text at the anchor changes,
     one polite notice says *the pane wrote to the row you are reading* — once, not once per
     frame — and the next thing the reader asks to hear is prefixed **"Changed."**. A row the node
     discarded outright is prefixed **"Those rows were discarded."** instead. The reader is never
     silently relocated and never quietly read stale text.
   - **Rule 2 stands down for the duration.** The cursor-line region is not composed while review
     is on. Two polite live regions on one surface, one of them firing on the pane's schedule and
     one on the reader's, is two voices talking over each other; the reader asked for one of them.
     Leaving review brings it back, and *back to the live cursor* is how you rejoin now without
     leaving.

   Walking off the top of the surface is answered rather than refused: the reader hears where the
   record stops and why — that it simply begins there, that older output was never captured
   because herdr hands back at most 1000 lines, or that *n* rows were discarded when output
   outran the poll.

## Consequences

**What this buys.** With TalkBack on and a real pane, an operator can now hear what the pane is,
where the cursor sits, how much history is behind it and whether that history is whole, whether it
has gone stale, and what the prompt currently says — and can type, because the key row's caps carry
semantics actions and the grid raises the keyboard. In the session behind probe #92 the caps went
from *unnamed and unpressable* to a double-tap that put characters in the shell. With rule 5 they
can also walk the whole surface a row and a word at a time, be told where history stops and what
was lost there, and stay where they parked while a build log runs underneath them.

**What it does not solve, and this list is the point:**

- **Review reads rows and words, and nothing else.** No column, no "read from here to the end",
  no character-by-character spelling of a row, no search. A two-column TUI is still read across
  both columns, which is the defect this ADR opened by naming; review makes it navigable, not
  columnar. Reading a column is the next thing worth building.
- **A soft-wrapped line is read as its separate rows.** Deliberate — see rule 5 — and still a loss
  on a pane full of prose, where rule 2's unwrapped line is the better unit and review cannot
  offer it.
- **A live row is anchored by position, not by content.** When the pane scrolls, the reader keeps
  the screen row and is told the text changed; they are not carried along with the line they were
  reading, because nothing in the wire says which row a line moved to. Following content across a
  scroll is the honest missing piece of rule 5.
- **Review is silent about everything except the row under the cursor.** A blocked prompt still
  announces itself, because that is a different live region, but output elsewhere on the grid is
  not mentioned at all while review is on. That is the trade this ADR keeps choosing, and it is
  still a trade.
- **A live region only speaks when its text changes**, so re-reading an identical row would be
  silence. An unspoken no-break space rides along on alternate reads to force the announcement.
  It works on TalkBack; it is a trick, and any reader that speaks that character breaks it.
- **A full-screen TUI is legible but not understood.** `htop`, `vim`, `less`, an ncurses installer
  can now be walked row by row. What no amount of navigation supplies is the structure — which
  rows are a header, which are a table, where the selection is.
- **No braille, and no cursor routing.** Compose Multiplatform exposes no braille surface, and
  nothing lets a reader route the terminal cursor by naming a cell — including from review, whose
  cursor cannot be handed to the pane.
- **Selection is sighted-only.** The handles are dragged at pixel positions; *Copy the selection*
  is named, but nothing can set the range without sight, and review cannot set it either.
- **Speech lags and drops.** A fast build log speaks the last settled line, not every line. That
  is deliberate — the alternative is speech that never completes — and it is still a loss.
- **History that is gone stays gone.** Rule 1 tells the truth about the hole; it does not fill it.
  `pane.read recent` caps at 1000 lines and takes no offset (probe #51), so nothing on either side
  of the wire can page further back.
- **The reasoning is from how console screen readers behave, not from blind terminal operators
  using Kampr.** No such user has tried it. That was the weakest joint in this ADR before the
  review mode existed and it is the weakest joint now — review was designed from the same reading
  of Orca, NVDA and BRLTTY, and one session with somebody who lives in those tools would say more
  about it than every paragraph above.

**What it costs to run.** One `LogicalText.lineAt` per 450 ms of quiet rather than per frame, and
while review is on, one `rowAt` per `revision` — a single row read against the scratch buffer, not
a walk of the ring. The walk backwards over soft-wrapped rows is bounded by the wrap chain.

## What would justify revisiting

- **A blind operator actually using this.** One session would outrank every paragraph above, and
  now there is a mode for them to have an opinion about.
- **Reading a column.** Review has a cursor and a surface to move it over; a column mode is a
  navigation decision on top of what already exists rather than a new subsystem.
- **Braille support in Compose Multiplatform.** A braille display is the one device that makes a
  cell grid genuinely legible, because it is spatial too.
- **A signal marking part of the grid as a document.** If a harness or herdr could say "these rows
  are prose", rule 1 could exempt that region and read it, and review could offer the logical line
  there instead of the row.
- **A wire change that survives a scrollback gap.** Rule 1 currently reports the hole honestly
  because there is nothing else to do about it. A per-segment `from_top` or a gap sentinel row
  would let review walk across one instead of stopping at it.

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
- **Re-reading the row aloud when the pane repaints it.** The same firehose narrowed to one row,
  which is better and still wrong: on a scrolling build log every settle window would speak
  whatever had just landed at that screen position, and the reader would be read at continuously
  while trying to read. One notice and a prefix on the next requested read carries the same fact
  and leaves the floor to them.
- **Anchoring the review cursor to a raw surface index.** The index is not stable — history
  arriving lengthens it from the top, a discard shortens it — so this is precisely the silent
  relocation rule 5 exists to prevent. It is also the version that would have shipped if the
  anchor had been written before the failure mode was understood.
- **A permanent badge saying how deep the ring is and whether it is whole.** A mark that is on
  every pane forever is a mark nobody reads, and it would spend chrome on the case where nothing
  is wrong. The description carries it only when something is; the surface carries it only where
  the record actually stops.
