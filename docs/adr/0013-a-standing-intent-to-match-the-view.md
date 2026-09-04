# 0013 — A standing intent to match the view

- **Status:** Accepted
- **Date:** 2026-09-03
- **Amends:** [ADR 0012](./0012-one-deliberate-resize-behind-a-panel.md), which is not superseded —
  its op, its floor, its measurement rule and its panel all stand unchanged. This adds a second
  way to *ask for* that op, and nothing else.
- **Evidence:** probes [#17–#21, #84, #219, #221, #233, #284, #298, #452](../03-probe-log.md)

## Context

ADR 0012 opened exactly one path that reshapes a pane: `pane.size`, behind a panel, with a
confirmation and an 80x24 floor. It also wrote down, in point 1, what makes that safe — *"No watch,
no zoom, no pan, no fit, no reconnect and no layout reaches it."* Rule 3 in `CLAUDE.md` states the
same thing from the other side: **nothing reshapes a pane because somebody looked at it.**

Both of those are about *why* a write happens, not about how many times the operator has to press a
button. And the operator, who is this project's only user, put the gap plainly:

> lets default wasm desktop/CLI to matching the views size and 'Hold while open', maybe if the view
> is above a certain size? obviously for mobile we should skip without an explicit request but for
> large sized screens its annoying to not

The panel already does this. What it does not do is *remember* that the operator wanted it. On a
desk-sized browser window, every pane opened is a panel opened, a hold ticked, a size chosen, and a
release remembered on the way out — for an answer that is the same every time and that the client
could work out from the window it is already measuring.

The thing that makes this legitimate rather than a violation is the same thing that made `pane.size`
legitimate: **the trigger is the operator, not the look.** What is being added is a *standing
intent* — a setting, discoverable and reversible, whose default depends on how big the viewport is —
and the write that follows it is 0012's op, unchanged, with 0012's floor.

### Why this is not "resize on view switch"

The write Rule 3 forbids is the one nobody chose: a fit, a reconnect, a layout, a re-watch. Those
are still forbidden and nothing here reaches them. Four properties keep the distance:

| | |
|---|---|
| It is a **setting**, with a switch on the same panel the resize lives on | An operator who did not want it turns it off and it stays off for that pane, on that device |
| It is **defaulted by measurement**, not by platform | The gate is the viewport the client already measured. A phone never claims; a mosaic cell never claims; a split half never claims |
| It **only ever means the operator's own desk** | The terminal surface holds; the conversation and mosaic surfaces never claim anything, and switching a pane from terminal to conversation releases |
| It **puts the pane back** | Which is the one thing the panel's hold has never done, and it is what answers [#298](../03-probe-log.md) |

### Why the terminal surface alone

The operator's own clarification, and it is load-bearing:

> 'viewer' should be terminal - if im looking at conversation pane and stay on it (likely on mobile)
> i wouldn't want it to change the desktop viewport if i switch to it

A conversation is reflowed prose; it has no column count worth imposing on anybody. A mosaic cell is
a thumbnail. Only the terminal surface renders a grid whose width being wrong is the complaint, and
only a desk-sized one of those.

## Decision

**`pane.size` gains a fourth mode, `match`: a hold owned by the websocket session that asked for
it.** It is the same claim, the same controller, the same floor and the same measurement rule. Four
things are new.

1. **The hold has an owner, and the owner is a socket.** A matched hold is a *lease*, and the lease
   lives in the client session's own state. It is released when the lease is dropped, and the lease
   is dropped on every path out of a session — the ordinary close, a `break` in the dispatch loop,
   the keepalive giving up on a peer that froze rather than closed
   ([#284](../03-probe-log.md)), and the whole task being cancelled when the node stops. **A closed
   laptop is a socket that stops answering**, and that is what ends the hold rather than anything
   the client remembered to send.

   This is why a matched hold carries **no wall-clock ceiling**, where the panel's hold is bounded
   by `HOLD_LIMIT`. That ceiling exists because a client that dies with the panel open never sends
   the untick ([#20](../03-probe-log.md)); a lease has no such gap, and a clock would only ever fire
   on an operator who was still sitting there looking at the pane. The remaining hole — the node
   being `SIGKILL`ed with a controller parked — is the same hole `HOLD_LIMIT` has, since that timer
   dies with the process too.

2. **Newest holder wins, and the loser does not fight back.** A later matched claim displaces an
   earlier one. The earlier viewer goes on rendering to fit, as it does today: zoom, pan, and the
   conversation view. *"you might have something open on your phone but also on your desktop, if you
   switch between the two and both are set to 'match view' that's kind of expected."*

   **The absence of a loop is structural rather than lucky.** A claim is edge-triggered by the
   client's own view — it opens, it closes, its window changes size — and never by anything the node
   reports about the pane. And the size asked for is computed at a **fixed reference cell**, so it
   is a pure function of the window's pixel size. A pane getting wider changes the earlier viewer's
   *zoom* and cannot change the number it would ask for. There is nothing for two viewers to
   alternate over.

   **Which cell is fixed, though, is the operator's answer and not always the base one.** The loop
   this guards against is the fit ladder's: while the zoom is *derived* it is a function of the
   pane's width, so a grid measured in those cells would move the pane, move the zoom, and ask
   again — there the reference has to be the base cell. A zoom the operator picked is not a function
   of anything; it is a constant, and measuring the window in its cells is exactly as pure. Using
   the base cell for both is what made the promise false: a pane held at 131x32 and drawn at 1.2x
   is a pane the window shows 26 rows of, and the operator's report was *"it fits the pane but it
   leaves a few blank lines at the bottom and i need to scroll up to see the claude logo top"*. So
   the reference cell is the base cell until a zoom is chosen and the chosen cell afterwards, which
   is the only reading under which "match this view" is true.

   A displaced lease is also *scoped*: it names its own hold by token, so the displaced viewer's
   release lands on nothing rather than taking the newer viewer's hold down with it.

3. **Release puts the geometry back, unless something else moved it.** The geometry found before the
   first claim is recorded and carried across every re-claim and every handover, so dragging a
   window twice does not make the size Kampr set the size Kampr restores. On release the pane is
   asked whether it still reads back the rows the hold put on it — `viewport_rows` is the PTY's own
   and herdr answers it honestly ([#84](../03-probe-log.md)) — and if it does not, **nothing is
   written**. Something else owns the pane now, and the one thing Kampr must not do to it is put a
   size on it nobody asked for ([#298](../03-probe-log.md)).

   The "found" geometry is honest or it is absent. Rows come from `viewport_rows`; columns come from
   a wrap the node has actually measured, because the layout rect is fiction
   ([#68](../03-probe-log.md)) and no method on the socket API reports a column count anywhere
   ([#221](../03-probe-log.md)). **A pane that has never wrapped has no width worth putting back**,
   and putting the rect back would be a resize to a number no row was ever laid out at. So such a
   pane records no restore at all, keeps the viewer's size when the view closes, and is moved from
   there by the panel like any other pane.

   The 80x24 floor does **not** apply to the restore. That floor exists so Kampr can never leave a
   pane too small to use ([#219](../03-probe-log.md)); putting back the size a pane was found at
   cannot do that, and refusing would strand it at the viewer's size instead.

4. **The default is decided by the measured viewport.** `Breakpoint.Desktop` — at least 900 dp wide
   and 600 dp tall — **and** a grid of at least 80x24 in the reference cell above, which is to say
   in the cells the operator is actually reading in. A desk zoomed to 1.6x shows 81x20 and therefore
   asks for nothing: the alternative is holding a pane at a grid nobody can see, which is the defect
   the paragraph above describes wearing a floor. Both conditions, because the first
   is the "this is a desk" signal and the second is what makes it honest: below the floor the node
   would refuse the claim anyway, so a view that small must never ask. The terminal surface measures
   its own box rather than the window's, so a split half, a mosaic cell and a phone in landscape all
   fall out on the same test. Below that line matching is off and stays off until the operator turns
   it on for that pane.

**A fleet pane is untouched, and needs to be.** Rule 3 already says a pane Kampr forked for a job of
its own is Kampr's: `kampr-fleet` gives it a geometry when the run starts and there is no operator
desk to trample. It is also not a herdr pane, so `parse_target` refuses `pane.size` on one outright
and always has. The client does not offer matching on one.

### Consent, and not nagging

The panel's confirmation names the consequence before anything is claimed, and it fires on the
operator's *first* claim of a pane — not on every automatic one. A confirmation on every view open
would be the nag that makes the feature worse than the panel it replaces, and a confirmation nobody
reads is worse than none.

So: **the switch is the consent, and it is on the panel the confirmation is already on.** The
resize panel carries a "match this view" toggle that states what it costs, and it is stored per
pane per device alongside `zoom` and `view` — which means an operator who turns it off has turned it
off, and an operator who never opens the panel gets the default their screen size chose. The pane's
own controls say while it is on that the pane is being held at this view's size, which is the
answer to *"a geometry change they did not ask for and cannot find the switch for"*.

## Consequences

- **The wire is additive.** A new `mode` value (`"match"`), one new optional request field
  (`lease`), and three new optional ack fields (`matched`, `found_cols`, `found_rows`). An older
  node answers `bad_request` to an unknown mode; an older client never sends one and ignores fields
  it does not know.
- **A held pane costs a `herdr terminal session control` child for as long as the view is open**,
  and while it is held the desk at that machine renders wrong without being told
  ([#18](../03-probe-log.md), [#298](../03-probe-log.md)). That is the cost the operator is buying,
  and it is why the switch says so and why the release puts the pane back.
- ADR 0012 point 3 — *"Holding is opt-in, and bounded"* — is amended in both halves for this mode
  only: the opt-in is a setting rather than a per-use tick, and the bound is a socket rather than a
  clock. The panel's own hold is unchanged.
- **Over a mesh link the guarantee is weaker.** A hub's link to a peer outlives any one phone
  looking through it, so a hub client's lease sends a scoped release down the link when it drops.
  That covers a cancelled hub session; it does not cover the hub process being killed, where the
  peer's hold is bounded only by its own mesh link failing.
- **A release is a resize, so it is not sent the instant a view ends.** The view that asks for a
  hold is not the thing that owns it: a pane switch is one terminal view leaving the composition and
  another arriving, so releasing on the way out put the found geometry back on the pane being left
  and the viewer's geometry on the pane being opened — and switching back wrote both again the other
  way round. The operator, on 0.1.57: *"switching panes now bounces around"*. The lease is therefore
  held by the client **session** (`MatchHolds`), which lets go `MATCH_LINGER_MS` after the last view
  of a pane ends and cancels that if another asks for the same pane meanwhile; a pane already held at
  exactly the grid being asked for is not claimed again, because a re-claim supersedes the controller
  and herdr shows the desk's own geometry in the gap. Point 3 is unchanged in substance — the pane is
  still put back, and still only if it still reads back the rows the hold put on it. The operator
  ticking the switch off is an *answer* rather than a view ending and is not given the window.
- `ARCHITECTURE.md` §4.2's "there is exactly one width the node does not have to infer, and it is
  the one it commanded" now covers this mode too — a matched hold *is* the geometry
  ([#18](../03-probe-log.md)), so its width is recorded as a proof exactly as `hold` already does.

## What would justify revisiting

- **Herdr shipping per-client geometry** (upstream U3), which deletes this whole ADR along with
  0012 rather than narrowing it. Still the outcome to pursue.
- **A controller that can decline geometry.** It makes the hold harmless and the restore
  unnecessary.
- **Evidence that a pane was resized by a viewer who did not know it was on.** 0012 said the same
  thing about its panel and it is more load-bearing here, because the trigger is a default: if this
  ever surprises an operator, the default is wrong, not the panel.
- **A second viewer's hold being taken by an earlier one's release.** The scoping is what prevents
  it; a report of it is a bug in the token, not a reason to widen the release.
