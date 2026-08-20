# 0004 — Scrollback is stitched from `pane.read`, and a gap discards rather than splices

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `37d6487` (stitching and the discard), `0954217` (adaptive polling)
- **Evidence:** probes [#25, #26, #27, #28, #29→#51, #30, #51, #55, #71](../03-probe-log.md)
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

1. **Alt-screen panes have no ring.** `max_offset_from_bottom` is 0 for Claude, Codex, vim
   (#30) — `recent` degrades to the viewport. Agent panes lose nothing by this: the conversation
   view is a better history than a ring, being whole-session and structured
   ([ADR 0005](./0005-structure-comes-from-the-transcript.md)).
2. **A read on an idle *recognised agent* pane can move the operator's screen.** Collie documented
   this hazard and it is respected rather than re-tested: `recent` with `lines > viewport_rows`
   harvests through the agent's own mouse-scroll interface. Hence the interlock — read only when
   `max_offset_from_bottom > 0` **and** the pane has no detected agent.
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

**On a gap, the old rows go.** If output outran the poll, the new read shares no overlap with the
ring, so the two stretches of history are *not adjacent* and nothing can prove what sits between
them. Splicing them would make `from_top` and `total_rows` fiction — a client would render two
unrelated stretches as one continuous document and have no way to know. So the node drops what it
held, **advances `from_top` by the number of rows dropped so absolute indices stay true**, and sets
`capped: true`.

**A width change restarts the ring for a different reason**, and the log says which happened: every
stored row was wrapped at the old PTY width, so nothing older can be trusted to line up.

**Polling is adaptive, because a fixed interval is not good enough.** The interval is
`clamp(row_budget / measured_rows_per_second, 100 ms, 2 s)` with a 400-row budget, so the cadence is
derived from Herdr's cap as a bound rather than guessed. A sweep test asserts that at every
unclamped interval fewer than 1000 rows land between reads. An idle pane is not polled at all — the
poller waits on a notify fired per frame with a 30 s backstop, so quiet panes cost nothing.

**The rate estimate comes from rows actually appended by the previous stitch, not from frame
content.** Herdr coalesces a burst to end state, so counting newlines in frames under-counts by
three orders of magnitude.

**Preserving history across a gap needs a wire change and is deliberately not in v1.** It would take
either a per-segment `from_top` or a gap sentinel row, and both should be specified before anyone
implements them.

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
