package dev.kampr.terminal

import dev.kampr.shared.wire.PaneModes
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.input.ScrollKeys
import dev.kampr.terminal.input.land
import dev.kampr.terminal.input.paneScrollKeys
import dev.kampr.terminal.input.paneTakesClicks
import dev.kampr.terminal.input.paneWheelBeatsTheRing
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// docs/26: what a wheel or a click sends is chosen by what the program asked for, and the program's
// name only says how far a report moves it.
class PaneModesTest {
    private val claude = PaneModes(mouse = "press", mouseSgr = true, altScroll = true, altScreen = true)
    private val less = PaneModes(altScroll = true, appCursor = true, altScreen = true)
    private val shell = PaneModes(altScroll = true)
    private val legacyMouse = PaneModes(mouse = "press", altScroll = true, altScreen = true)

    private val scrolls = listOf(
        Triple("claude's own request", "claude", claude) to ScrollKeys.Wheel,
        Triple("muffit's", "muffit", claude) to ScrollKeys.Notch,
        Triple("a program nobody measured that took the mouse", "codex", claude) to ScrollKeys.Report,
        Triple("any program with no agent that took the mouse", null, claude) to ScrollKeys.Report,
        Triple("drag tracking", "claude", claude.copy(mouse = "drag")) to ScrollKeys.Wheel,
        Triple("motion tracking", "foo", claude.copy(mouse = "any")) to ScrollKeys.Report,
        Triple("less on the alternate screen", null, less) to ScrollKeys.CursorKeys,
        Triple("vim named as an agent", "vim", less) to ScrollKeys.CursorKeys,
        Triple("the mouse beats alternate scroll", "foo", claude.copy(altScroll = false)) to ScrollKeys.Report,
        Triple("alternate scroll switched off", null, less.copy(altScroll = false)) to null,
        Triple("alternate scroll with no alternate screen", null, shell) to null,
        Triple("the legacy mouse form is sent nothing", "claude", legacyMouse) to null,
    )

    @Test
    fun theProgramsOwnRequestPicksTheKindAndTheNameOnlyTheRowsAReport() {
        for ((case, want) in scrolls) {
            val (what, agent, modes) = case
            assertEquals(want, paneScrollKeys(agent, cmd = "whatever", modes = modes), what)
            assertEquals(want, paneScrollKeys(agent, cmd = null, modes = modes), "$what, with no job named")
        }
    }

    // What the cmd gate was for: a shell at its prompt has set no mouse mode and is not on the
    // alternate screen, whatever label outlived the harness.
    @Test
    fun aPaneWithNoMouseAndNoAlternateScreenIsSentNothingWhateverItsAgentSays() {
        for (agent in listOf(null, "claude", "muffit", "codex", "vim")) {
            for (cmd in listOf(null, agent ?: "bash")) {
                assertEquals(null, paneScrollKeys(agent, cmd, PaneModes()), "$agent / $cmd")
                assertEquals(null, paneScrollKeys(agent, cmd, shell), "$agent / $cmd, alternate scroll left on")
                assertEquals(false, paneTakesClicks(agent, cmd, PaneModes()), "$agent / $cmd clicks")
            }
        }
    }

    // The permanent fallback: an older node, or a herdr pane, says nothing and the tables decide as
    // they always did.
    @Test
    fun aPaneWithNoModesKeepsTheNameTables() {
        assertEquals(ScrollKeys.Wheel, paneScrollKeys("claude", "claude", modes = null))
        assertEquals(ScrollKeys.Notch, paneScrollKeys("muffit", "muffit", modes = null))
        assertEquals(ScrollKeys.CursorKeys, paneScrollKeys(null, "less", modes = null))
        assertEquals(null, paneScrollKeys("claude", null, modes = null))
        assertEquals(true, paneTakesClicks("claude", "claude", modes = null))
        assertEquals(false, paneTakesClicks("claude", null, modes = null))
        assertEquals(false, paneTakesClicks("codex", "codex", modes = null))
        assertEquals(false, paneTakesClicks(null, "less", modes = null))
    }

    // herdr labels a muffit TUI pane nothing and names its job `muffit-tui` however it was started
    // (#699), and cursor keys into muffit's composer recall the operator's input history.
    @Test
    fun muffitsTuiIsKnownByItsJobWhenNothingLabelsIt() {
        assertEquals(ScrollKeys.Notch, paneScrollKeys(agent = null, cmd = "muffit-tui", modes = null))
        assertEquals(ScrollKeys.Notch, paneScrollKeys(agent = null, cmd = "muffit-tui", modes = claude))
        assertTrue(paneWheelBeatsTheRing(agent = null, cmd = "muffit-tui", modes = null))
        assertFalse(paneWheelBeatsTheRing(agent = null, cmd = "claude", modes = null), "a classic Claude's ring is its conversation")
        assertFalse(paneWheelBeatsTheRing(agent = null, cmd = "bash", modes = null))
        assertEquals(ScrollKeys.CursorKeys, paneScrollKeys(agent = null, cmd = "claude", modes = null), "Claude is tuned by its label alone")
    }

    @Test
    fun aClickGoesToAProgramThatTookSgrMouseReportingWhateverItsName() {
        assertEquals(true, paneTakesClicks("claude", "claude", claude))
        assertEquals(true, paneTakesClicks("codex", "codex", claude))
        assertEquals(true, paneTakesClicks(null, "vim", claude.copy(mouse = "drag")))
        assertEquals(false, paneTakesClicks("claude", "claude", less), "no mouse mode: nothing is listening")
        assertEquals(false, paneTakesClicks("claude", "claude", legacyMouse), "the legacy form is sent nothing")
    }

    @Test
    fun aReportMovesAProgramOneRowAndTheFingerAndTheWheelAgree() {
        val sent = mutableListOf<String>()
        PaneScroll(ScrollKeys.Report) { sent += it }.wheel(rows = -4, col = 9, row = 2)
        assertEquals(List(4) { "\u001b[<65;10;3M" }, sent)
        assertEquals(5, ScrollKeys.Report.land(count = 5, reversed = true).rows, "no ramp and no loss on a reversal")
    }
}
