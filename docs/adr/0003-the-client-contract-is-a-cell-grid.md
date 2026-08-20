# 0003 — The client contract is a cell grid, not ANSI

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** `f1f8860` (spec), `1be3bff` (encoder), `836f1a0` (per-connection tables)
- **Evidence:** probes [#36, #37, #41, #56–#62](../03-probe-log.md); `crates/kampr-core/tests/wire.rs`
- **Depends on:** [ADR 0001](./0001-the-node-runs-a-vt-emulator.md)

## Context

[ADR 0001](./0001-the-node-runs-a-vt-emulator.md) settles that a VT emulator runs in the node. That
leaves a separate question, and it is the one Collie's ADR 0008 refuses even inside a hypothetical
emulator: **what does the node send a client?**

There are three candidates.

**Forward the ANSI.** The node emulates in order to hold state, and also relays Herdr's frames
onward. Every client then needs its own emulator to apply the diffs — a second emulator in Kotlin,
in `commonMain`, doing exactly what the Rust one already did, and disagreeing with it in the usual
ways. It also means Herdr's frame format reaches a phone, so Herdr's serialiser becomes Kampr's
public API and every Herdr release is a client release.

**Rendered rows** — Collie's `StyledLine[]`. Wrapped text with SGR reattached. This is the honest
answer if the product is triage and the pane is context below the fold. It is the wrong answer if
the product claims a live terminal, because it discards the cursor, the column grid, and every
structural fact a terminal has that a paragraph does not.

**A cell grid.** Style-interned, run-length-encoded rows plus a cursor and a link table.

Collie refused the third on a specific ground worth quoting, because it is the strongest objection
and it is not about rendering at all:

> **A cell-grid wire protocol is refused even inside a hypothetical emulator.** The phone shows ~50
> of a ~200-column pane, so a column-faithful grid reopens the pan-vs-wrap question 0.21.0 settled
> toward wrap.

That is correct, and it is a product decision rather than a technical one. Collie settled pan-vs-wrap
toward wrap; a cell grid would reopen it. Kampr settled the same question toward **pan**, in
[ADR 0002](./0002-kampr-never-resizes-a-pane.md) and in every layout decision downstream of it, and
that is a deliberate divergence rather than an oversight. Once pan is the answer, the objection is
spent: a column-faithful grid is exactly what a pan-and-zoom surface wants.

Three measurements decided the rest.

**Fidelity has somewhere to go.** A frame-fed emulator reproduces Herdr's own grid 30/30 rows with
the cursor on the right cell, and recovers a hyperlink that `pane.read` drops (#41, #36, #37). Rows
would throw that away at the last step, after having gone to the trouble of getting it right.

**A cell grid costs less on the wire than intuition suggests.** Style interning plus run-length
encoding measured **61×** smaller than per-cell JSON at 124×50 with 49 distinct pens, and **44×** at
74×30 with light colour. A full grid is a few kilobytes. The regression test asserts a floor of 10×
on a deliberately run-hostile pattern.

**A cell grid is cheap to draw, and rows would not have been cheaper.** The client rendering spike
holds 60 fps at 74×30 on real WebGL2 and on Android, with fill effectively free and text shaping the
entire cost (#56, #58). Shaping cost is driven by how much text there is, not by whether it arrived
as cells or as rows — so the wrapped-rows alternative would have paid the same bill and bought less.

## Decision

**The node ships clients a cell grid, and clients parse no escape sequences.**

- `styles` is a **per-connection, append-only** table. Style `0` is always the default pen and is
  seeded before the first message, so a client can render before it has been told anything. Each
  message carries only the suffix a client has not seen.
- `grid.reset` carries every row; `grid.patch` carries only the rows that changed. A `Run` is
  `{s: style_id, x: text, l: link_id?}`, contiguous from column 0, with trailing default cells
  omitted and padded by the client.
- **`links` is a delta and may appear on `grid.patch`.** A hyperlink can first be seen mid-stream.
  Clients append in arrival order; ids are indices.
- **`row` is `u32`, not `u16`**, because the same row type carries absolute scrollback indices and a
  deep ring overflows 16 bits.
- **Unknown `t` values and unknown fields are ignored, never errors.** That rule is the entire
  forward-compatibility story, and it runs in both directions.
- **The wire protocol is Kampr's, and Herdr's frame format stops at the node.** Write clients
  against [`../04-wire-protocol.md`](../04-wire-protocol.md), never against Herdr.

## Consequences

- **Kampr owns a protocol, and owes it a version story.** `hello.protocol` exists and is currently
  parsed and never read. There has been one migration and no story for a second. That is a debt this
  decision created.
- **Two encoders now exist, and a third is arriving.** The node encodes for a client; the mesh's
  `shadow` module decodes a peer's stream at the hub and re-encodes it per client, so each client
  keeps the style ids it was told about. Every hop pays an encode. That is the price of
  per-connection tables, and it buys the property that a client never sees an id it was not given.
- **A backpressure purge cannot be uniform.** History is append-only and nothing repairs a hole in
  it, and a purged style entry orphans runs that survive it. So only `grid.reset` and `grid.patch`
  are purgeable; `scrollback` and `styles` never are. This was a real defect before it was a rule —
  a catch-all classification made scrollback purgeable, which is a permanent unsignalled hole in
  history and effectively a row cap of exactly the kind the design had just ruled out (`836f1a0`).
- **Clients get structure for free.** Zoom and pan are pure rendering over a cell model. Selection
  can be linear or block. A link id resolves through the pane's table. None of that is reachable
  from wrapped rows.
- **The client became a renderer with a performance budget rather than a text view.** That is the
  whole subject of [ADR 0008](./0008-two-render-modes-not-a-glyph-atlas.md), and it is a cost this
  decision incurred.
- **Both sides of a seam can agree with each other and still disagree with the wire.** Three wire
  bugs of exactly that shape shipped and survived their own tests: `hello.security.unlocks` was a
  sentence where the client wanted a list, `herd.patch` emitted arrays and decoded objects, and
  `scrollback.total_rows` was a depth on one side and an absolute end index on the other — which
  agreed perfectly until a ring first discarded, at which point the client would have drawn a
  thousand phantom rows (`d11260f`). A protocol you own is a protocol you have to test by bytes.

## What would justify revisiting

- **A second client that cannot render a grid.** A terminal-based Kampr client, or an integration
  that wants text, is better served by a rows *projection* of the cell model than by changing this
  contract. The grid is the richer form and rows derive from it; the reverse is not true.
- **Wire cost becoming the bottleneck on a real cellular link.** The compression numbers are against
  per-cell JSON, not against a binary encoding. If bytes ever matter more than debuggability, the
  answer is a binary framing of the *same* model — not a different model.
- **A per-segment `from_top` or a gap sentinel for scrollback.** Named here because it is the one
  known, specified, deliberately-deferred addition to this protocol.
  See [ADR 0004](./0004-scrollback-is-stitched-and-a-gap-discards.md).
