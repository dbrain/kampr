# 0004 — Scrollback is stitched from `pane.read`, and a gap discards rather than splices

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `37d6487` (stitching and the discard), `0954217` (adaptive polling)
- **Evidence:** probes [#25, #26, #27, #28, #29→#51, #30, #51, #55, #71, #529, #530, #532](../03-probe-log.md)
- **Depends on:** [ADR 0002](./0002-kampr-never-resizes-a-pane.md)

## Context

A terminal you cannot scroll back in is not a terminal. Kampr has to supply history, and every
obvious source is closed.

**The frame stream cannot supply it.** This is the probe that removed the easy answer: `seq 1 200`
on a 30-row pane put **29 distinct lines across the entire frame stream** — the final viewport.
Lines 1–171 were never transmitted at all (#25). Herdr coalesces to grid state rather than replaying
a byte stream, which is exactly what makes the live path cheap (#23) and exactly what makes it
useless for history. A frame-fed emulator cannot rebuild scrollback, however long it has been
watching.

**`terminal.scroll` is not available.** It is a control-mode stdin command, and Kampr does not use
control mode ([ADR 0002](./0002-kampr-never-resizes-a-pane.md)). This is the price of that decision,
paid here.

**Herdr does hold the ring, and hands it over cheaply.** `pane.read source:"recent"
format:"ansi"` on a shell pane returned 401 lines in **0.002 s** with the viewport unmoved and all
256-colour SGR intact (#27, #28). That is the source.

It comes with three hard edges, and the third only appeared after code was built on a wrong reading
of the second:

1. ~~**Alt-screen panes have no ring, so agent panes lose nothing.**~~ **Half withdrawn (#231).**
   The measured half stands: a pane genuinely on the alternate screen reports
   `max_offset_from_bottom: 0` and `recent` degrades to the viewport, and exiting alt screen
   restores the ring (#30). What was extrapolated from it — that a detected agent pane *is* such a
   pane — is false. A live `codex` read back **402** rows of ring and a live `claude` **384**; the
   one harness that really does clear its scrollback is Claude Code once it has taken the screen.
   So agent panes do have history to lose. The conversation view is still the better history for
   them ([ADR 0005](./0005-structure-comes-from-the-transcript.md)), but it is no longer the only
   one they have.
2. **A read on an idle *recognised agent* pane can move the operator's screen — and it is gated on
   the read's own parameters, not on the pane (#513, superseding #231).** herdr harvests through the
   agent's mouse-scroll interface for `format: "text"` with `source: "recent"`/`"recent_unwrapped"`
   and `lines` above the screen's row count, on a pane whose detected agent is **idle** and which
   holds the **alternate screen** with mouse reporting on — 5.3 seconds of injected wheel events,
   measured on 0.9.0 and identically on 0.8.2. #231 tested a live `codex` and a live `claude` that
   were **blocked** and holding an ordinary ring, so not one of those conditions held; on that state
   `lines: 5000` really does answer in 1 ms with the viewport unmoved. What was wrong was the
   generalisation. **So the interlock is not what makes this safe — the format is.**
   `read_scrollback` asks for `format: "ansi"`, which is outside the gate, and a test named for the
   defect fails if that changes. The interlock itself stays as `max_offset_from_bottom > 0`, which
   is what excludes the one read that *is* slow: a live harness whose ring is empty.
3. **Reads cap at 1000 lines and there is no offset parameter.** Probe #29 originally recorded that
   over-asking clamps harmlessly with `truncated: false`; that only held because the ring under test
   was 400 deep. Against a 1371-row ring, `lines=5000` returns **1000** with `truncated: true`, and
   `pane.read` takes no offset — so **deeper history cannot be paged to at all** (#51). Herdr's
   `truncated` means "there was more than you asked for", not "we hit the cap" (#55).

That third edge is what this ADR is really about. A node that only ever reads once is capped at 1000
rows forever. A node that *watches* is in a better position: successive reads overlap, and the
overlap is proof of adjacency.

## Decision

**The node accumulates a ring past Herdr's cap by stitching overlapping reads, and when two reads
share no overlap it discards what it held rather than splicing across the gap.**

The stitch is the longest suffix of what is held that is also a prefix of what just arrived; only
the remainder is appended. Proven live at 1553 rows — every one of them above what a single read can
return — with all 1600 markers accounted for and colour intact.

**On a gap, the old rows go — and since herdr 0.9 that is the *fallback* rather than the answer.**
See the amendment below. The reasoning stands wherever the rows cannot be refetched: two stretches
of history that nothing can prove adjacent must not be spliced, because `from_top` and `total_rows`
would become fiction and a client would render them as one continuous document with no way to know.
So the node drops what it held, **advances `from_top` by the number of rows dropped so absolute
indices stay true**, and sets `capped: true`.

**The stitch itself was also unsound, and that is not about gaps at all.** A suffix match on output
that repeats itself finds a run that is not the true one: measured, a ring holding herdr's rows
0..361 met a window starting at 3403 and answered `Stitched { added: 599 }`, rendering
`complete: true, capped: false` across a 3041-row hole (#522). That is the shape #233 taught this
project to fear — not a read that failed, but one that failed and then looked like one that worked.
Where a position is available the join is now arithmetic and the suffix match is the fallback.

**A width change restarts the ring for a different reason**, and the log says which happened: every
stored row was wrapped at the old PTY width, so nothing older can be trusted to line up.

**Polling is adaptive, because a fixed interval is not good enough.** The interval is
`clamp(row_budget / measured_rows_per_second, 100 ms, 2 s)` with a 400-row budget, so the cadence is
derived from Herdr's cap as a bound rather than guessed. *(Superseded: the budget is **8** and is
sized by how far behind the grid a reader's history may be; the cap is a consequence of it. See the
2026-09-11 amendment.)* A sweep test asserts that at every
unclamped interval fewer than 1000 rows land between reads. An idle pane is not polled at all — the
poller waits on a notify fired per frame with a 30 s backstop, so quiet panes cost nothing.

**The rate estimate comes from rows actually appended by the previous stitch, not from frame
content.** Herdr coalesces a burst to end state, so counting newlines in frames under-counts by
three orders of magnitude.

**Preserving history across a gap needs a wire change and is deliberately not in v1.** It would take
either a per-segment `from_top` or a gap sentinel row, and both should be specified before anyone
implements them. *(Overtaken: the rows are refetched instead, so there is no gap left to describe —
see the amendment.)*

## Amendment — 2026-09-08: the gap is refilled, and the join is arithmetic

herdr 0.9's `pane.selection.read` addresses history by **absolute row** (#510), which is the offset
`pane.read` never had. Two things follow, and the first matters more than the second.

**The join stopped being a guess.** A read now carries where it starts — `L - K + 1`, where `L` is
the pane's last content row, which is *not* `bottom` whenever the tail is blank (#518, #519). The
ring remembers where its last row sits, so continuing and gapping are told apart by arithmetic. That
closes #522's false stitch, which no amount of tuning could have.

**A gap is refilled rather than discarded.** The missing span is fetched in one call — 4303 rows in
2 ms — re-split at the grid width in **display cells**, because `selection.read` unwraps soft wraps
and, after a reflow, joins rows that never wrapped; splitting by character count instead mismatched
61 rows out of 63 on CJK (#521). Measured end to end: 262 held rows and 3041 unseen ones recovered,
`complete: true`, against a baseline of `Gap { dropped: 262 }` (#523).

**The discard is still there, and it is still right.** herdr's absolute row numbers are positions in
its *current* ring, not identities — a retention trim renumbers them wholesale and reports nothing
about how far (#520) — so the fetched rows are only spliced when eight of them demonstrably continue
the ring's own tail. Measured refusing on exactly that case, with herdr trimmed underneath the node
(#523). A refusal falls through to the behaviour this ADR describes.

**What it costs.** One `pane.read visible` per poll to locate `L`; one `selection.read` per gap.
Refilled rows are **plain text** — no SGR, no OSC 8 (#510) — so a repaired span renders unstyled.
That is a real fidelity loss, and it is a loss of styling on rows that would otherwise not exist at
all.

## Amendment — 2026-09-11: the cadence is sized by the reader, not only by the cap

**The poll's budget was a cap margin doing a display job badly.** `clamp(row_budget /
rows-per-second, 100 ms, 2 s)` with a 400-row budget reads a pane producing forty rows a second
*twice a second*, and the socket sat on a three-second floor of its own on top of that. The client
draws the grid pinned directly under the last history row it holds, so for the span of both, the
rows that had left the live grid were in **neither half of the surface** — missing from it
outright rather than blank — and arrived later in one batch. Measured on a real pane at that rate,
the worst seam was **158 rows** (#529). The operator, on a phone: *"it will scroll through but the
history will skip lines and then update. so there will be gaps in the data that will get filled."*

It is invisible on a desk, where a pane matched to the view has viewport ≈ live grid and the seam
falls off-screen. On a phone it is the top of the screen: #526 measured that pane holding 52
viewport rows against a 30-row live grid.

**Both halves were load-bearing and neither alone was enough** — budget 8 with the old floor still
seams at 121 rows, the old budget with a 100 ms floor at 80, and the two together at **10** (#529).

**What made the floor necessary was the lay-out, and the lay-out is now incremental.** Rendering the
ring is linear in its whole depth — 55 ms at the 20 000-row bound (#529) — and it was paid on every
read, which is exactly what a three-second floor was buying back. It is sound to lay out only the
rows just appended because **herdr re-emits every row's styling on the row itself**: a colour set
once and spanning three lines comes back as three independently styled rows, and so does an
unclosed bold (#530). A row laid out alone is the row it would have been laid out as in company.
The one thing that still invalidates the cache is a row wider than every row before it, because the
grid is sized by its widest row.

So the budget is the **display** bound now — how far behind the grid a reader's history may be —
and staying under herdr's 1000-row cap is a consequence of it rather than its purpose, which is a
strictly larger margin than before. A socket is handed only the rows it has not had, so a send
costs what the pane just produced instead of everything it has ever produced.

**What is still true, and is the open item this does not close.** The join is *narrower*, not
honest: nothing on the wire says where the live grid's row 0 sits in the ring's index space, so the
client still assumes adjacency and is still wrong for the width of one poll.

**Measured across rates (#532), which settles both halves of it.** The seam is **10 rows at 40/s,
14 at 125/s, 1303 at ~500/s and 8431 unthrottled** — so the cadence above covers an operator's
ordinary work and a burst walks straight past it, which is the same arithmetic against the 1000-row
cap this ADR already concedes for `from_top`. `capped` is true at both burst worst-cases and does
**not** cover it: `capped` speaks about the *top* of the ring, and the client draws its loss label
there, while the unmarked join is at the bottom where the reader is.

**And the sentinel cannot be made exact.** A `recent` read always reaches the live edge, so at the
moment of a read the ring's last row *is* the grid's top row minus one: the hole is zero by
construction and opens only between reads, where nothing but herdr knows how far the pane scrolled.
A node can narrow it by reading more often — which is what the amendment above does — or estimate
it from the measured row rate, but it cannot measure it, and a field carrying a number the node
computed at the one moment it is always zero would be worse than none. So the gap sentinel as
specified is not the fix for *this* seam; what it was specified for is preserving history across a
discard, which is a different quantity. **The open item is therefore narrower than it was**: whether
a client should decline to draw history adjacent to the grid while the pane is producing faster than
the ring can follow, which is a rendering decision needing no wire change and no number.

## Consequences

- **`capped: true` is an honest statement, not a failure mode.** It means "the top of this ring is
  not the top of history". A client must not present it as complete.
- **History that scrolled away before the node started watching is unreachable, permanently.** No
  amount of asking again helps; `pane.read` returns the newest 1000 every time.
- **A single verbose command still loses history.** Probe #71: `seq 1 40000` in a watched pane
  produced a first read of 962 rows already `capped`, and the next read shared no overlap, so the
  ring restarted and `from_top` advanced. The cap is not a corner case.
- **Adaptive polling narrowed the window but did not close it.** A sustained thousand rows per
  second now survives with `from_top` unchanged; a four-thousand-row *instant* burst still gaps.
  That is arithmetic against a 1000-row cap, not a tuning failure, and it is evidence *for* the
  per-segment wire change rather than against the poll.
- **Two bugs here were only visible in a live run**, and both made the adaptive poll worse than the
  fixed one it replaced. A pane with no ring yet is indistinguishable from an alt-screen pane, and
  parking both missed the entire first burst — the gate answers from cached state, so asking costs
  nothing. And a single zero-row sample erased the estimate, because terminal output is bursty
  enough that a fast poll lands in a lull and relaxes just in time to gap; the estimate decays now
  instead of resetting.
- **`scrollback` messages are exempt from a backpressure purge.** History is append-only and a
  `grid.reset` carries the viewport and nothing above it, so a purged scrollback message is a
  permanent unsignalled hole. That exemption is part of this decision, not part of the transport's.
- **The node's ring bound is 20 000 rows and is a memory limit, not a display one** — roughly 4 MB
  at ~200 bytes of raw ANSI per row. It is configurable, and **clients must not add a cap of their
  own.**

## What would justify revisiting

- **`terminal.scroll` on an observer, or a read-only input mode for `control`** — upstream ask U8.
  That returns real scrollback to the live view and makes most of this machinery unnecessary.
- **An offset parameter on `pane.read recent`** — upstream ask U8c. Deep history becomes reachable
  and the discard becomes rare rather than structural.
- **The gap sentinel, which should now be decided rather than deferred.** The original condition was
  "evidence that gaps still hurt after adaptive polling", and there is some: `docs/06-audit.md`
  argues the case, though its arithmetic uses the 3 s *geometry* poll rather than the 100 ms–2 s
  scrollback poll and so overstates the frequency. The honest statement is that a burst faster than
  the estimator can react to still discards, probe #71 saw it in ordinary use, and a two-field wire
  addition would preserve what is currently thrown away. This is the most defensible open item in
  the protocol.
