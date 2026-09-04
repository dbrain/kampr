package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val ROW = 5
private const val COL = 11

private class ClickIo(
    private val agent: String? = "claude",
    private val cmd: String? = agent,
    override val readOnly: Boolean = false,
) : PaneIo {
    val typed = mutableListOf<String>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) typed += msg.text
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) =
        PaneInfo(paneId, "node", agent = agent, cmd = cmd, cols = 94, rows = 40)
}

// A harness holding the whole screen: rows of its own chrome, nothing under the tap that reads as a
// path or a link, and no ring behind it — which is every pane on the alternate screen (#438).
private fun panel(): PaneState = paneShowing(
    *Array(12) { row -> if (row == ROW) "  files changed since HEAD                    ✕" else "  claude" },
)

private const val HISTORY = 60

private fun withRing(pane: PaneState): PaneState {
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = Phone.PANE,
            fromTop = 0,
            rows = (0 until HISTORY).map { RowDiff(it, listOf(Run(0, "history $it"))) },
            totalRows = HISTORY,
            complete = true,
            capped = false,
        ),
    )
    return pane
}

// A desk that fits inside the test window, so a cell five rows down is a cell a finger can reach.
// `deskTerminal`'s 1440x900 is laid out inside a 1024x768 root and the rows below the fold are
// painted at negative offsets — real geometry, unreachable pointer.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.desk(pane: PaneState, session: PaneSession, io: ClickIo) =
    gridTerminal(pane, session, io, width = 1000.dp, height = 740.dp, bars = SafeArea.None)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tapped(io: ClickIo, pane: PaneState = panel(), above: Int = 0): List<String> {
    val session = PaneSession(Phone.PANE)
    desk(pane, session, io)
    tapCell(session, above + ROW, COL)
    return io.typed
}

// The report: *"there's a code diff thing that pops up … it has a little 'x' but tapping it does
// nothing"*. The panel is Claude Code's own `/diff`, drawn inside the pane, and its ✕ is a control
// the program presses when a mouse report arrives on that cell — Kampr forwarded the wheel to these
// panes and nothing else, so a tap on a control the operator could see was the one gesture with
// nowhere to go (#480).
//
// **Only a tap that found nothing of Kampr's own.** A path or a link under the finger still opens
// the card it always did; this is what the tap did when it did nothing but raise the keyboard.
@OptIn(ExperimentalTestApi::class)
class ClickingIntoAPaneTest {
    @Test
    fun aTapOnAHarnessThatTakesOneIsAClickWhereItLanded() = runComposeUiTest {
        val typed = tapped(ClickIo())
        assertEquals(
            listOf("\u001b[<0;${COL + 1};${ROW + 1}M", "\u001b[<0;${COL + 1};${ROW + 1}m"),
            typed,
            "the tap did not reach the program as a press and a release on its own cell",
        )
    }

    // The gate that keeps a shell out of it. `cmd` is null at a prompt and null when nothing could
    // tell, and an SGR report at a readline prompt is not ignored — it is typed in as characters.
    @Test
    fun aPaneWithNoHarnessOnItIsNeverClickedInto() = runComposeUiTest {
        assertEquals(emptyList(), tapped(ClickIo(agent = null, cmd = null)))
    }

    @Test
    fun aPaneBackAtItsPromptIsNeverClickedInto() = runComposeUiTest {
        assertEquals(emptyList(), tapped(ClickIo(agent = "claude", cmd = null)))
    }

    // Everything nobody has probed for the mouse takes the same answer the wheel gives it: nothing.
    // A report typed into a program that never asked for one is bytes at its prompt.
    @Test
    fun aHarnessNobodyHasProbedIsNeverClickedInto() = runComposeUiTest {
        assertEquals(emptyList(), tapped(ClickIo(agent = "codex")))
    }

    @Test
    fun aReadOnlyDeviceClicksNothing() = runComposeUiTest {
        assertEquals(emptyList(), tapped(ClickIo(readOnly = true)))
    }

    // The coordinates are the live grid's, and a pane Kampr holds a ring for has rows above row
    // zero that are not on the program's screen at all — so the cell under the finger is not a cell
    // the program could be told about.
    @Test
    fun aPaneKamprHoldsHistoryForIsNeverClickedInto() = runComposeUiTest {
        assertEquals(emptyList(), tapped(ClickIo(), withRing(panel()), above = HISTORY))
    }

    // The path is still the path. A tap that found one of Kampr's own targets opens the card it
    // always opened and sends nothing, or the file viewer would be unreachable on exactly the panes
    // that have the most paths on them.
    @Test
    fun aTapOnAPathStillOpensTheCardAndClicksNothing() = runComposeUiTest {
        val io = ClickIo()
        val session = PaneSession(Phone.PANE)
        desk(paneShowing(SHOWN), session, io)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        assertTrue(session.view.target != null, "the tap found no target, so nothing is being tested")
        assertEquals(emptyList(), io.typed, "a tap that opened a card also clicked into the pane")
    }
}
