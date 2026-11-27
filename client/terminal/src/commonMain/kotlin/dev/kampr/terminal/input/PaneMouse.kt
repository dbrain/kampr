package dev.kampr.terminal.input

import dev.kampr.shared.wire.PaneModes

// Harnesses measured to act on an SGR left-button report, for a pane whose node cannot say what the
// program asked for (a herdr pane, #292, #686, or an older node): a harness nobody has probed is not
// guessed at. Claude Code 2.1.260 presses the control under the cell — the ✕ on its `/diff` panel
// closes it (#480).
//
// A pane that has told its modes (docs/26) is not asked by name: a program that took SGR mouse
// reporting is sent the click, and one that did not is sent nothing, whatever its `agent` says. The
// operator can turn Claude Code's own click handling off with `CLAUDE_CODE_DISABLE_MOUSE_CLICKS`,
// which leaves it setting `?1000h` and ignoring every report anyway (#480): sending one there is
// measured to be inert, which is what makes the modes safe to follow in that direction.
private val TAKES_A_CLICK = setOf("claude")

// **Without modes, `cmd` is the gate and it fails closed**, exactly as `paneScrollKeys`' is. It is
// null both at a shell prompt and when nothing could tell, and a report typed at a bare readline
// prompt is not ignored — it is *typed*: `echo A` plus a press becomes `echo A0;10;3M`, because
// readline eats `ESC [ <` and takes the rest as characters (#480). A harness label outlives the
// harness, so the label alone would still be clicking at a prompt a minute after the agent quit.
fun paneTakesClicks(agent: String?, cmd: String?, modes: PaneModes? = null): Boolean = when {
    modes != null -> modes.tracksMouse && modes.mouseSgr
    else -> cmd != null && agent != null && agent in TAKES_A_CLICK
}

// SGR 1006, button 0, at 1-based cell coordinates: the press, then the release on the same cell.
// **Both, and both at the same cell.** A press alone does nothing and is not what fires a control;
// a release whose cell differs from the press's fires nothing either (#480).
fun clickReports(col: Int, row: Int): List<String> = listOf(
    "\u001b[<0;${col + 1};${row + 1}M",
    "\u001b[<0;${col + 1};${row + 1}m",
)
