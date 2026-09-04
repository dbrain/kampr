# 0011 — The client fits itself to the pane, and asks the operator before it touches a pane's mouse

- **Status:** Accepted
- **Date:** 2026-08-28
- **Shipped in:** _(set at the release commit)_
- **Evidence:** probes [#291](../03-probe-log.md), [#292](../03-probe-log.md),
  [#293](../03-probe-log.md), [#298](../03-probe-log.md), [#299](../03-probe-log.md),
  [#300](../03-probe-log.md)
- **Depends on:** [ADR 0002](./0002-kampr-never-resizes-a-pane.md),
  [ADR 0003](./0003-the-client-contract-is-a-cell-grid.md)

## Context

`kampr` with no arguments is now a terminal client of its own herd. Unlike the phone and the
browser, it runs **inside** a terminal that has its own geometry, its own colour palette and its
own idea of what a mouse is — three things every other Kampr client owns outright. Each of them
turned out to be a decision, and two of them read as violations of ADR 0002 unless the reasoning
is written down.

**A pane is routinely wider than the terminal looking at it.** A headless PTY measures 93 columns
([#68](../03-probe-log.md)) and a zoomed one reaches 292 on this desk. ADR 0002 forbids the obvious
answer: Kampr may not reshape the pane. The plan that produced this client called the inverse —
resize the *terminal* to fit the pane, via XTWINOPS `CSI 8;rows;cols t` — *"the inversion that
makes the whole thing cheap"*, and assumed it would work.

**It does not work here, and the failure is not uniform.** [#291](../03-probe-log.md): ghostty
1.3.1 — the operator's own terminal — ignores `CSI 8;rows;cols t` entirely, and so does kitty
0.48.2. konsole 26.04.3 honours it in under 5 ms with one SIGWINCH, and **does not clamp**: asked
for 400x900 on a 2560x1440 display it reported a 7200x6000 px text area. So the rung is
unavailable on two of three emulators and actively dangerous on the third.

**Nothing on herdr's socket says whether a pane's program wants the mouse.**
[#292](../03-probe-log.md): observe frames carry exactly four private modes — `?2026h/l` and
`?25h/l` — and DECSET 1000/1002/1003/1006 never appear, however the pane's program asks for them.
That is structural rather than a gap: observe emits grid state, not a byte replay
([#23](../03-probe-log.md)/[#25](../03-probe-log.md)), so a mode change is consumed by herdr's own
emulator and has nothing left to be re-emitted as. `pane.graphics.info`'s `pixel_mouse` is a decoy
that reads constant `true` and describes the *host* client. [#293](../03-probe-log.md) closes the
neighbouring question the same way: the alt screen is equally invisible, so there is no
disambiguator to infer from either.

**And a viewing side effect can silently wreck somebody's screen.**
[#298](../03-probe-log.md) measured `herdr terminal session control` against an attached desk TUI:
it neither refuses nor evicts, it reshapes the PTY underneath a client that goes on drawing the old
box. A 69-column line came back whole from the API and appeared at the desk **cropped at 49**, with
no error anywhere. This is ADR 0002's invariant with a visible victim, and it is why the two
decisions below are conservative in the same direction.

## Decision

**1. Geometry: the client fits itself, by a ladder that asks and verifies, and never derives.**

1. The terminal is already wide enough — draw it.
2. Ask the terminal to resize itself, and **check whether it did**. Name the host with
   `CSI >0q`; compute the largest grid the display can hold from `CSI 14t` divided by `CSI 16t` —
   **not** from `TIOCGWINSZ`, whose pixel fields go stale in konsole while `14t` stays honest
   (#291); **refuse the request ourselves** when the pane is wider than that; write it; re-read
   `TIOCGWINSZ`; and treat anything past **50 ms** as a refusal.
3. Crop and pan. **On this desktop this is the path, not the fallback.**

The ladder **says which rung it used and why**, including *rung 2 was refused by this terminal* and
*rung 2 was not tried — nothing answered*. Those are different states: a terminal that refused was
asked, and a terminal that answered nothing gave no display size to clamp against, so asking it
would be requesting a window nothing bounds.

**No rung reshapes a pane.** ADR 0002 is untouched: `terminal.resize` appears nowhere in this
crate, and the one thing that moves is the operator's own window.

**2. The mouse: chrome is ours, a pane is the operator's to arm.**

Clicks on Kampr's own chrome — tabs, sidebar rows, the herd view, a prompt's option chips — are
client-side and unconditional. Clicks *into* a pane are sent only for a pane the operator has
explicitly armed, remembered per pane in `prefs`, and stated in the status line whenever it is on.
A pane running a recognised program may be **offered** the toggle; it is never flipped for them.

**The graphical client answers this differently, and the difference is a measurement.** It has no
status line to state a mode in and no chord to arm one with, and it forwards the *wheel* to a
harness measured to take one already ([#388](../03-probe-log.md)) — so a tap that found none of
Kampr's own targets is forwarded as a click on the same terms: a per-harness table, never a
heuristic on `cmd`, and `cmd == null` refusing outright so nothing is ever typed at a prompt. What
makes that safe is the second half of [#480](../03-probe-log.md): a report sent to a Claude Code
that is not listening leaves the screen byte-identical, so the table is safe to be wrong about in
the only direction it can be wrong. The TUI's arming stands where it is — there the operator has a
keystroke and a status line, and this client has neither.

**3. Colour: the emulator keeps its own 16 slots.** `Default` and `Indexed(0..=15)` pass through as
ordinary SGR so the operator's terminal skins them; `Rgb` and `Indexed(16..=255)` pass verbatim.
ADR 0009's concern — absolute values staying absolute — is preserved, while the thing the operator
already themed stays theirs. Kampr's own chrome uses the Phosphor tokens so the three clients agree
where it matters.

## Consequences

- **The common case on this desk is crop-and-pan**, so that rung carries the felt quality of the
  client and gets the engineering — smooth panning, a position indicator, keys to the edges.
- **Rung 2 is nearly dead code on the machines it was written on**, kept because it is cheap,
  self-answering, and correct for konsole and for terminals nobody here can measure. It cannot fire
  wrongly: it clamps itself first.
- **A mouse-aware program does not "just work"** the way it does under `herdr --remote`, which is
  herdr's own TUI on the PTY and therefore knows the mode. One keystroke arms it. That is a worse
  experience than the desk and it is the honest one: the alternative is typing `[<0;10;5M` into
  somebody's shell.
- **A heuristic on `cmd` is weaker than it looks**: [#297](../03-probe-log.md) measured that under
  ble.sh — every interactive shell on the operator's own machine — `pane.process_info` reports only
  `bash` while a job runs. Suggesting on a recognised command is sound; failing open on an
  unrecognised one is not.
- **The client must reset what it set.** Bracketed paste and mouse reporting are modes the shell
  does not clear, and crossterm's `EnableMouseCapture` turns on five of them including 1003
  any-motion and 1015 urxvt ([#300](../03-probe-log.md)). They are released by an RAII guard and a
  panic hook rather than on the clean exit path, which is one of six ways out.

## What would justify revisiting

- **herdr putting the pane's mouse mode on the wire.** [#292](../03-probe-log.md) is a statement
  about herdr 0.8.2, not a law. A `pane.get` field, or a mode change in an observe frame, and the
  arming step becomes unnecessary — the client would encode a click for the mode the pane declared.
  This is the single change that would most improve the client.
- **XTWINOPS becoming reliable**, or the operator moving to a terminal that honours it. Rung 2 is
  already built and self-verifying; nothing needs to change but the answer it gets.
- **A measured `~/.terminfo`-style capability source** that beats `CSI >0q`. Note
  [#299](../03-probe-log.md) before trusting any name table: konsole answers kitty's *graphics*
  query and renders kitty graphics, so the table written from #291 was stale on the machine that
  produced it. In-band probes have beaten names twice here.
- **Evidence that arming is a nuisance in practice.** If operators arm every pane immediately and
  never regret it, the default is wrong — but that is a report from use, not a thing to assume.
