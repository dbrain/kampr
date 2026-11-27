# Pane modes: choosing what to send a program from what it asked for

A wheel notch, a drag on a phone, a click: what Kampr sends to a pane's program depends on what that
program is. Today that is decided by name. `TAKES_THE_WHEEL` in `client/terminal/.../PaneScroll.kt` and in
`crates/kampr-tui/src/mouse.rs` lists `claude` and `muffit`; `TAKES_A_CLICK` in `PaneMouse.kt` lists `claude`.
Everything else gets cursor keys. muffit was missing from the list, and its TUI took the arrows as
"previous message" in its composer: the wheel recalled the operator's input history instead of scrolling
(fixed by adding it to the list, 2026-11-27). Every new harness repeats that bug until somebody notices.

The program already says what it wants, in DEC private modes. This brief makes the pane's modes part of
the wire and lets the client follow the rule every terminal follows.

## The rule

For one wheel notch (or the equivalent travel of a finger):

1. The program enabled mouse tracking (`?1000`, `?1002` or `?1003`): send a mouse report. SGR form
   (`ESC [ < 64|65 ; col ; row M`) if it also set `?1006`; otherwise the legacy `ESC [ M` form, which
   caps at column and row 223 (send nothing past that rather than a wrong cell).
2. Else, on the alternate screen with `?1007` on: cursor keys, application form (`ESC O A`/`B`) when
   DECCKM (`?1`) is set, else `ESC [ A`/`B`. Today's `CursorKeys` is always the application form,
   because that moved `less` where the normal form moved nothing (#390); keep that until a probe says the
   DECCKM-aware form is no worse.
3. Else: nothing is sent, and the client scrolls its own ring if the pane has one.

A shell at its prompt has no mouse mode and is not on the alternate screen, so it reaches rule 3 with no
help from `cmd`. That makes the "`cmd` is the gate and it fails closed" rule redundant for scrolling, and
it fixes the stale label (`agent` outliving the harness) for free: programs reset their modes on exit.
Clicks follow rule 1 with a button-1 report. `TAKES_A_CLICK` goes the same way, but Claude's click
handling is a product decision the operator can turn off (`CLAUDE_CODE_DISABLE_MOUSE_CLICKS`), and the
gate there was about not clicking at a prompt a minute after the agent quit; modes cover that.

## What stays per program

How many reports, not which kind. Claude ramps (+0.3 rows per report landing within 40 ms of the last,
ceiling 6, and the first report after a change of direction moves nothing, #567–#572), and `PaneScroll`'s
drag pacing is tuned to it. muffit moves a fixed three rows a report (`ScrollKeys.Notch`). A table keyed by
agent keeps those two: `rowsPerReport` / ramp. A program not in it gets one row per report, which scrolls
correctly if not smoothly. So the table is a tuning list, and forgetting a program degrades smoothness
rather than breaking the scroll.

## Wire (additive only)

An optional `modes` object on the pane's `grid.reset` (and re-sent whenever any of these change, as a
`grid.modes` message, or on `pane.info`: pick whichever the existing mode-like fields already use; read
docs/04 before choosing):

```jsonc
{ "mouse": "none" | "press" | "drag" | "any",   // ?1000 / ?1002 / ?1003
  "mouse_sgr": true,                              // ?1006
  "alt_scroll": true,                             // ?1007
  "app_cursor": false,                            // ?1
  "alt_screen": true }                            // ?1049 / ?47 / ?1047
```

Absent means a node that predates it, and the client keeps today's name-based behaviour for that pane:
`paneScrollKeys` takes `modes` as an optional argument and falls through to the tables when it is null.
That fallback is permanent: an older node behind a hub is the normal case. `wire-shapes.txt` is blessed
with the new paths (`KAMPR_BLESS_WIRE=1`) and docs/04 gets the field.

## Where the modes come from, which is the open question

- **Native sessions**: `kampr-native`'s emulator (alacritty `TermMode`) already tracks `MOUSE_*`,
  `SGR_MOUSE`, `ALTERNATE_SCROLL`, `APP_CURSOR` and `ALT_SCREEN`, and the checkpoint carries them
  (`screen/checkpoint.rs`), so a replay rebuilds them. Straightforward.
- **herdr panes**: unknown. `herdr_provider.rs:592` notes that "herdr says nothing of a pane's 2004
  (#667)", so herdr's API does not report modes. Whether the node's own emulator for a herdr pane (the
  byte stream from `herdr terminal session observe`) sees `?1000h`/`?1006h`/`?1007h` is what to measure
  first: run Claude, muffit, `less` and `vim` in a herdr pane and read the node's emulator state. Append
  a probe row (rule 1) with what was found. If the stream does not carry the program's mode sets
  (herdr may swallow them), herdr panes stay on the name tables, and that is a documented limit.
- **Peers and hubs**: the hub relays `grid.reset` by pane id, so a new optional field passes through; an
  older hub that rebuilds frames drops it, and the client falls back.
- **muffit's TUI in a terminal pane** is a native session, so it is the first case covered.

## Order of work (TDD)

1. Probe: herdr pane modes as above. Record the row.
2. `kampr-core`: the `PaneModes` type and its wire shape, with the wire-shape sample.
3. `kampr-native`: read the modes out of the emulator for a native pane; a node test that Claude-like
   output (`?1000h`, `?1006h`) and `less`-like output (`?1049h`, `?1007h`) produce the expected
   `modes`, and that a program exiting (`?1000l`) clears them.
4. Client (`PaneScroll.kt`, `PaneMouse.kt`) and `kampr-tui` (`mouse.rs`): choose by modes first, table
   second. The existing table-driven tests (`InputTest.kt` `PaneScrollTest`, `pointer.rs`
   `the_wheel_over_a_pane_with_no_ring_scrolls_the_program_in_it`) gain a `modes` column; the cases with
   `modes = null` must still pass unchanged (the permanent fallback).
5. A test that a pane with `mouse = none` and `alt_screen = false` is sent nothing whatever its `agent`
   says, which replaces the `cmd == null` stale-label test's job.
6. Mutation-check each (restore the defect, watch it fail, #191).

## Risks

- A program that crashes with mouse tracking left on gets wheel reports typed into the shell. Every
  terminal does this; `reset` fixes it. It is why the node should clear modes when the foreground job
  changes to a shell prompt, if it can tell.
- Mode changes race the gesture: a program that has just enabled mouse mode and a wheel already in
  flight. The cost is one notch sent in the old dialect.
- Legacy (non-SGR) mouse reports are rare; if no program measured needs them, send nothing for that case
  and say so, rather than ship an encoder nobody tests.
