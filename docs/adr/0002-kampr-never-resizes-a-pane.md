# 0002 — Kampr never resizes a pane

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `f1f8860`; restated in every brief since
- **Evidence:** probes [#13–#21, #31–#34, #52, #68](../03-probe-log.md)

## Context

A pane is a shape somebody chose at a desk. The obvious thing for a phone client to do is change it
— narrow the pane to the phone's width, and the reflow problem disappears. This is the single most
tempting decision in the product, and it reversed twice during design before the probes settled it.

Herdr offers exactly one way to do it. `herdr terminal session control` accepts `terminal.resize`
against the pane's real PTY, and it is also the only input channel that carries `terminal.scroll`.
So "let Kampr resize" and "let Kampr scroll the terminal like a terminal" are the same decision,
bought with the same coin.

What the probes found about that coin:

- **`control` always claims the PTY, and there is no flag to decline.** With no `--cols`/`--rows` at
  all it still takes the pane to 120×40 (#17). There is no read-only-input control mode. The
  ownership is not a feature you can opt out of; it is what `control` *is*.
- **While a controller holds it, the person at the desk is ignored.** The desktop client was resized
  to 120×44 during a control session; the PTY stayed at the controller's size until release (#18).
- **A frozen controller holds it forever.** `SIGSTOP` on the controller kept the pane at 16 rows
  indefinitely — the socket is still open, so Herdr has no reason to reclaim (#20). Release and
  `SIGKILL` both restore geometry within a second (#19), but a wedged process is exactly the failure
  a phone on a flaky link produces.
- **Geometry is a shared, last-writer-wins global.** Two clients at 100×30 and 60×20 produced an
  18-row pane; resizing the first to 200×50 took it back to 50 while the small client was still
  attached (#31). There is no smallest-wins negotiation to hide behind.

So the pane a phone reshapes is the pane on the desk. Not a copy of it, not a view of it — the same
PTY, running the same processes, redrawing itself at a phone's dimensions while somebody is looking
at it. A `vim` session reflowed to 40 columns by a phone that then loses signal is a wedged editor
on a monitor two rooms away.

The alternative Kampr took costs one capability and nothing else:

- **`observe` never touches the PTY.** An observer at 60×20 ran against a 36-row pane and the PTY
  stayed 36 (#14). Many observers coexist, each at its own requested geometry (#13).
- **Input does not need the terminal stream at all.** `pane.send_text` and `pane.send_keys` are
  ordinary one-shot socket calls with no ownership and no session state, and `send_text` writes raw
  bytes — verified against `cat -v`, which echoed `^[`, `^[[5~`, `^[[H`, `^A` and UTF-8 intact (#9).
  Reading and writing are independent surfaces.

The capability given up is `terminal.scroll`, which is control-only. That is what makes
[ADR 0004](./0004-scrollback-is-stitched-and-a-gap-discards.md) necessary and awkward.

## Decision

**Kampr never resizes a pane, and `terminal session control` is not used in any form.**

- Rendering uses `observe` at the pane's own geometry. Small screens are a rendering problem — zoom,
  pan, and the conversation view — not a geometry problem.
- Input goes over JSON-RPC (`pane.send_text` / `pane.send_keys`), never over a terminal stream.
- **There is no `resize` message in the wire protocol and there will not be one.** The node cannot
  reshape a pane, so a client cannot ask it to.

**This is structural, not a policy.** Nothing in the node holds a lease, negotiates geometry, or
decides not to resize. The code path does not exist. That distinction matters because a policy is
something a later feature quietly exempts itself from, and a structure is not.

**The invariant is about side effects of viewing, not about refusing to act.** `pane.split`,
`workspace.create` and the rest of the `manage` surface change pane geometry for everyone, because
that is what those actions mean at the desk too. They are explicit operator requests and Kampr
performs them; the UI's obligation is to say what a structural action will do before doing it. What
this ADR forbids is reshaping somebody's session *as a consequence of looking at it*.

## Consequences

- **A phone gets the pane's real width.** 844 px of landscape fits 94 columns at 13 px, so landscape
  is a first-class layout rather than a rotation fallback. Portrait is pan-and-zoom over a real grid.
- **`terminal.scroll` is unavailable, so scrollback comes from `pane.read` instead** — with a
  1000-row cap, no offset parameter, and an honest gap when output outruns the poll. That whole
  chain of compromise is downstream of this decision.
  See [ADR 0004](./0004-scrollback-is-stitched-and-a-gap-discards.md).
- **`observe --cols` crops rather than reflows** (#15), so the observed geometry must be the pane's
  actual geometry or rows are silently truncated. Getting it wrong is not a cosmetic error.
- **Native geometry is poll-only.** No Herdr event fires when the desk resizes a pane — six event
  types, three verified resizes, zero events (#52). The node polls the layout rect every 3 s and
  restarts the observer on a change, which costs one `full` frame. `layout.updated` covers
  structural change only.
- **The layout rect is not always the truth.** In a headless session — the configuration both the
  plugin and the service produce — the PTY does not follow the layout rect at all (#68), and nothing
  in the socket API reports a pane's real column count. The node therefore *infers* it: `pane.read
  recent` returns rows already wrapped at the PTY width and `recent_unwrapped` returns the logical
  lines they came from, so a logical line longer than the widest physical row proves a soft wrap
  happened, and a soft wrap happens at exactly the PTY width. Without a wrap there is only a lower
  bound, which is why it combines with the rect by `max`. Sizing an observer is the one place where
  this system reasons from evidence rather than from a reported number.
- **Two people on two phones can type into the same pane at once, and that is correct.** Input is
  stateless, so there is nothing to arbitrate. For a personal tool that is the right answer; for a
  shared one it would need thinking about.
- **Kampr can show panes the Herdr TUI cannot show together.** Nothing binds a view to one server,
  so a single window can hold panes from several sessions on several hosts. A TUI client attaches to
  exactly one server and structurally cannot. This is a direct dividend of never owning geometry.

## What would justify revisiting

- **Herdr adding per-client geometry that does not resize the shared PTY** — upstream ask U3, and the
  general form of U2 (reflow rather than crop in `observe --cols`). That removes the entire trade:
  a phone-shaped view with the desk untouched. This is the outcome to *pursue*, because it is the
  only one that makes both sides right.
- **A read-only input mode for `control`, or `terminal.scroll` on an observer** — upstream ask U8.
  That would return scrollback to the live view without returning geometry ownership, and would
  substantially simplify ADR 0004 without touching this one.
- **Nothing about a better phone UI.** Zoom being awkward, a wrapped line being ugly, a user asking
  for it — none of these reopen the question, because none of them change what `control` costs
  the person at the desk. The reason to revisit is upstream capability, not local frustration.
