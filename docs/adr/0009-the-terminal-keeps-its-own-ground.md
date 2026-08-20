# 0009 — The terminal keeps its own dark ground, and only its 16 slots are themed

- **Status:** Accepted
- **Date:** 2026-08-20
- **Shipped in:** _(set at the release commit)_
- **Evidence:** probe [#88](../03-probe-log.md); contrast figures below are computed from the
  shipped token values by `ThemeContrastTest`, which fails the build if they move
- **Depends on:** [ADR 0003](./0003-the-client-contract-is-a-cell-grid.md),
  [ADR 0005](./0005-structure-comes-from-the-transcript.md),
  [ADR 0008](./0008-two-render-modes-not-a-glyph-atlas.md)

## Context

Kampr now has a light ground for all four themes. The chrome moves cleanly — every screen resolves
through the token layer, so a light `Palette` per theme is the whole change. The terminal does not
move, and it is the reason this decision needs writing down rather than deciding itself.

**A cell's colour arrives in one of three forms, and Kampr can tell them apart.** The client
contract is a cell grid, not ANSI ([ADR 0003](./0003-the-client-contract-is-a-cell-grid.md)), so
each cell carries a discriminated `ColorSpec`:

| form | what it names | redirectable by a palette? |
| --- | --- | --- |
| `Rgb(r, g, b)` | an absolute sRGB value | **no** |
| `Indexed(n)`, n ≥ 16 | a fixed entry in the 6×6×6 cube or the greyscale ramp | **no** |
| `Indexed(n)`, n < 16, and `Default` | one of the 16 palette slots | **yes** |

That discrimination is the one advantage Kampr has over Collie, which reads a rendered buffer
downstream of the PTY and cannot see which form produced a pixel. It does not change the answer.

**A harness in a herdr pane cannot find out what background it is on.** Probe #88: `OSC 11` and
`OSC 10` both return zero bytes from a headless pane, while `CSI c` in the same script answers
`ESC[?62;22c` — the PTY replies to queries, it simply does not implement these two. `COLORFGBG` is
unset. `COLORTERM` is `truecolor`. So the terminal tells a program two things: *nothing* about the
ground, and *everything* about the colour depth. Both push the same way — a harness falls back to
dark and emits absolute values. Collie reached this independently and measured the consequence:
truecolour is 79–100% of what three of four harnesses emit.

**Dark-authored absolutes are unreadable on a light ground, and no palette can reach them.** The
two values in Claude's own output, against each theme's light `bg`:

| | soft `#f7f8fb` | phosphor `#f3f5ef` | warm `#faf6ef` | brutalist `#ffffff` |
| --- | --- | --- | --- | --- |
| `#f6e2b7` | 1.20 | 1.16 | 1.18 | 1.27 |
| `#abdfa7` | 1.43 | 1.38 | 1.41 | 1.52 |

Against the terminal ground this ADR keeps, the same two values sit at **15.25** and **12.81**
under soft. The colours are not the problem; the ground is.

## Decision

**The terminal surface carries its own ground — dark under every theme and under both app
grounds — and everything painted inside it comes from `TerminalPalette`, never from
`tokens.color`.**

Three rules follow, all load-bearing:

1. **The terminal ground is a per-theme value on the terminal skin, not `surface2`.** It was
   derived from the chrome palette; deriving it is exactly what would have made it flip. Soft is
   `#0b0d12`, phosphor `#050705`, warm `#15110d`, brutalist `#000000` — each recognisably its
   theme, none of them light.

2. **Anything drawn inside the terminal ground resolves through `TerminalPalette`** — the 16
   slots, the default ink, the selection wash, the link ink. Reaching for `tokens.color` inside
   the terminal is a bug that is invisible in dark and wrong in light, which is the worst
   combination a rule can have. This is the same trap Collie documents; the shape of the fix is
   different because Kampr's terminal is a `Canvas`, not a `<pre>`.

3. **The 16 slots are the only colours Kampr may redirect, and it redirects all 16 per theme.**
   Indices 16–255 and every `Rgb` pass through byte-exact: a program asking for `#f6e2b7` gets
   `#f6e2b7` under all four themes and both grounds. Semantics are fixed — 1 is error, 2 is
   success — and only hue and chroma move.

### Why not invert, which is what Collie chose

Collie renders its mirror into a `<pre>` and flips it with `filter: invert(1) hue-rotate(180deg)`.
Two things make that the wrong port:

- **Kampr paints cells onto a `Canvas`.** The equivalent is a `graphicsLayer` colour matrix over
  a scrolling surface, per frame, on a phone. This project already carries two render modes
  because shaping is 93% of frame cost ([ADR 0008](./0008-two-render-modes-not-a-glyph-atlas.md));
  a full-surface filter is precisely the per-frame cost that ADR exists to avoid, and it is
  unprofiled on the hardware that matters.
- **More decisive: the terminal is not Kampr's primary reading surface.** Collie's mirror *is*
  the product, so a permanently dark mirror leaves a dark slab where the reading happens. Kampr's
  reading surface for an agent is the transcript ([ADR
  0005](./0005-structure-comes-from-the-transcript.md)) — fully themed, light and dark, and what
  `AppState.openPane` selects by default when a pane has no ring. The terminal view is the raw
  fallback. A dark ground there is what an IDE does with its embedded terminal, and it reads as a
  device rather than as an oversight.

## Consequences

Per theme, the 16 slots against that theme's own terminal ground, and the same measurement against
the single shared table this replaces:

| theme | min | median | max | faint min | *was* min | *was* median | *was* faint min |
| --- | --- | --- | --- | --- | --- | --- | --- |
| soft | 5.18 | 11.28 | 17.78 | 3.38 | 2.87 | 11.08 | 1.64 |
| phosphor | 4.65 | 10.93 | 17.40 | 3.06 | 2.90 | 11.23 | 1.63 |
| warm | 4.68 | 8.58 | 16.44 | 3.16 | 2.84 | 10.99 | 1.63 |
| brutalist | 5.62 | 11.81 | 21.00 | 3.43 | 3.04 | 11.75 | 1.63 |

Slots 1–15 clear **AA body text (4.5:1)** in every theme; the old shared table failed it in all
four, at the slot agents use most — bright-black, the comment colour. **Slot 0 is exempt**: it is
the ground by convention, and lifting it to 4.5:1 would break every program that pairs `30m` with
an explicit background.

`SGR 2` (faint) moved from alpha 0.55 to **0.75**, which is what puts the faint form of every slot
above 3:1 instead of 1.63:1. Faint is still visibly faint; it is no longer a guess.

What it costs:

- **A dark slab in a light app.** The pane header, key row and column indicator around the
  terminal stay light, so there is a hard seam. That seam is the decision made visible, and it is
  worth watching: if it reads as breakage rather than as a device, the fix is a frame or an inset
  around the terminal, not a light palette.
- **A trap for contributors,** identical in shape to Collie's: every instinct inside the terminal
  — use the token, follow the ground — is wrong, and wrong in a way that compiles and looks
  correct in dark. `TokenLayerTest` keeps colour literals out of the terminal module, so a slot
  cannot be re-hardcoded there; `ThemeContrastTest` holds the AA floor and the passthrough. What
  neither catches is `tokens.color.accent` used as ink inside the ground.
- **A light-themed agent is unreadable, and this makes it no worse.** A harness configured for a
  light terminal emits dark ink, which is illegible on the dark ground under either app ground.
  Nothing on the wire carries the harness's theme, so Kampr cannot detect it. Pre-existing, and
  recorded because the fix — a per-pane "this one is light" flag — should be able to layer on.

## What would justify revisiting

- **Herdr answering `OSC 11`, or any herdr setting that supplies the ground to the pane.** That
  kills the premise: a harness could then author for light, and a light palette becomes reachable.
  Re-run probe #88 before assuming it still holds.
- **A wire field carrying "authored for dark/light" per pane.** One bit, not a colour map — the
  same conclusion Collie reached from the other side. A per-harness colour table is refused
  outright: it needs an entry per harness per release, breaks silently on a retune, and cannot
  cover the tools an agent runs.
- **A profiled colour-matrix invert on a mid-range ARM phone showing it is free.** The cost is the
  whole objection; remove it and the option reopens.
- **A measured real-output profile showing palette-dominated harnesses.** If the harnesses Kampr's
  users actually run draw mostly from the 16 slots, per-harness rendering — re-theme those, keep
  the rest dark — becomes arguable. Chrome-only counts do not qualify.

## Alternatives closed off

- **A light ANSI palette.** Reaches only the third row of the table above, which is the minority
  of what harnesses emit. Building it would produce a set that is correct and irrelevant.
- **Clamping absolute colours to a lightness floor.** Misrepresents what the program emitted, on
  values it named exactly, and the clamp point is arbitrary. It also breaks rule 3, which is the
  one promise the terminal makes.
- **Deriving the terminal ground from `surface2`.** How it worked before. It is one token away
  from flipping to white the moment a light palette exists, which is how this whole question
  arrived.
