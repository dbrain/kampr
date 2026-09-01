package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val WHEEL_UP = "\u001b[<64;"
private const val WHEEL_DOWN = "\u001b[<65;"
private const val KEY_UP = "\u001bOA"
private const val KEY_DOWN = "\u001bOB"

private class AgentIo(
    private val agent: String? = null,
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

// A pane with no ring behind it, which is the one in the report: a harness on the alternate screen
// draws one viewport and keeps nothing above it.
private fun noRing() = Phone.shell(rows = 8, caretRow = 3)

// The same pane with a ring, which is every pane that is *not* holding the whole screen. Nothing
// may be typed into one of these: Kampr has the history itself and the wheel is its own.
private fun withRing(): dev.kampr.shared.model.PaneState {
    val pane = Phone.shell(rows = 8, caretRow = 3)
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = Phone.PANE,
            fromTop = 0,
            rows = (0 until 200).map { RowDiff(it, listOf(Run(0, "history $it"))) },
            totalRows = 200,
            complete = true,
            capped = false,
        ),
    )
    return pane
}

// Enough notches to reach the end of whatever Kampr's own surface has, on any font metrics.
private const val TO_THE_END = 80

// A finger pulled down the screen asks for what is above it, which is the direction a wheel-up
// asks for. Several moves rather than one: the first of them spends whatever the surface had.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragDown(steps: Int = 10) {
    onRoot().performTouchInput {
        var y = 100f
        down(Offset(width / 2f, y))
        repeat(steps) {
            y += 150f
            moveTo(Offset(width / 2f, y))
        }
        up()
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheel(notches: Float) {
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        scroll(notches)
    }
    waitForIdle()
}

// The report, on 0.1.44: *"wasm desktop, some terminals i can't scroll up on, unclear why. others
// i can. the terminal is running claude. doesn't try to scroll at all"*. A harness on the alternate
// screen keeps no ring for herdr to hand over (#387), so there was nothing above the viewport and
// the wheel moved nothing at all — while the program in the pane was holding that history itself,
// and scrolls its own view on a wheel report (#388).
@OptIn(ExperimentalTestApi::class)
class PaneWheelTest {
    @Test
    fun aWheelOverAPaneThatKeepsNoRingGoesToTheProgram() = runComposeUiTest {
        val io = AgentIo("claude")
        val session = PaneSession(Phone.PANE)
        phoneTerminal(noRing(), session, io = io)

        repeat(TO_THE_END) { wheel(-1f) }
        assertTrue(io.typed.isNotEmpty(), "the wheel over a pane with no history sent nothing at all")
        assertTrue(
            io.typed.all { it.startsWith(WHEEL_UP) && it.endsWith("M") },
            "not SGR wheel-up reports: ${io.typed.first().drop(1)}",
        )

        // And back down, because a pane scrolled up by this path moved inside the program: nothing
        // on this side can bring it back.
        val up = io.typed.size
        repeat(TO_THE_END) { wheel(1f) }
        assertTrue(
            io.typed.drop(up).any { it.startsWith(WHEEL_DOWN) },
            "the notch the other way was never sent down",
        )
    }

    // The gate that keeps the two apart. A pane Kampr holds history for is a pane Kampr scrolls
    // itself, and typing an SGR report into one would be bytes at a shell prompt.
    @Test
    fun aPaneKamprHoldsHistoryForIsNeverTypedInto() = runComposeUiTest {
        val io = AgentIo("claude")
        val session = PaneSession(Phone.PANE)
        phoneTerminal(withRing(), session, io = io)

        repeat(TO_THE_END) { wheel(-1f) }
        assertTrue(session.view.scrollY > 0f, "the surface never moved, so the ring was not there")
        assertEquals(emptyList(), io.typed, "a pane with its own history was sent pty bytes")
    }

    // The phone's half of the same hand-off, and the reason it needs one: a finger has no wheel,
    // and on a pane holding the whole screen a drag ran out of surface and then did nothing.
    @Test
    fun aFingerDragPastTheEndOfTheSurfaceScrollsTheProgram() = runComposeUiTest {
        val io = AgentIo("claude")
        val session = PaneSession(Phone.PANE)
        phoneTerminal(noRing(), session, io = io)

        dragDown()
        assertTrue(io.typed.isNotEmpty(), "the drag ran out of surface and then did nothing at all")
        assertTrue(
            io.typed.all { it.startsWith(WHEEL_UP) && it.endsWith("M") },
            "a finger pulled down asked for the wrong direction: ${io.typed.first().drop(1)}",
        )
    }

    @Test
    fun aFingerOnAPaneKamprHoldsHistoryForIsNeverTypedInto() = runComposeUiTest {
        val io = AgentIo("claude")
        val session = PaneSession(Phone.PANE)
        phoneTerminal(withRing(), session, io = io)

        dragDown()
        assertTrue(session.view.scrollY > 0f, "the surface never moved, so the ring was not there")
        assertEquals(emptyList(), io.typed, "a pane with its own history was sent pty bytes")
    }

    @Test
    fun aPaneWithNoHarnessOnItIsNeverTypedInto() = runComposeUiTest {
        val io = AgentIo(agent = null, cmd = null)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(noRing(), session, io = io)
        repeat(TO_THE_END) { wheel(-1f) }
        repeat(TO_THE_END) { wheel(1f) }
        assertEquals(emptyList(), io.typed, "a pane with no harness on it was sent pty bytes")
    }

    // Everything that is not a measured harness takes the terminal's own default — the alternate
    // scroll herdr does at the desk — rather than nothing at all. `vim`, `less`, `man`, and an
    // agent nobody has probed for the mouse.
    @Test
    fun anythingElseHoldingTheScreenIsScrolledWithCursorKeys() = runComposeUiTest {
        for (program in listOf("vim", "less", "codex")) {
            val io = AgentIo(agent = if (program == "codex") program else null, cmd = program)
            val session = PaneSession(Phone.PANE)
            phoneTerminal(noRing(), session, io = io)
            repeat(TO_THE_END) { wheel(-1f) }
            assertTrue(io.typed.isNotEmpty(), "$program was given no way to scroll at all")
            assertTrue(
                io.typed.all { it == KEY_UP },
                "$program was sent something other than an application cursor key: ${io.typed.first().drop(1)}",
            )
            repeat(TO_THE_END) { wheel(1f) }
            assertTrue(io.typed.any { it == KEY_DOWN }, "$program was never sent the other direction")
        }
    }

    // The gate that keeps a shell out of it. A harness label outlives the harness, so a pane back
    // at its prompt would otherwise be typed into a minute after the agent quit.
    @Test
    fun aPaneWhoseForegroundJobIsUnknownIsNeverTypedInto() = runComposeUiTest {
        val io = AgentIo(agent = "claude", cmd = null)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(noRing(), session, io = io)
        repeat(TO_THE_END) { wheel(-1f) }
        dragDown()
        assertEquals(emptyList(), io.typed, "a pane that may have been at its prompt was typed into")
    }

    @Test
    fun aViewerThatMayNotTypeSendsNothing() = runComposeUiTest {
        val io = AgentIo("claude", readOnly = true)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(noRing(), session, io = io)
        repeat(TO_THE_END) { wheel(-1f) }
        assertEquals(emptyList(), io.typed, "a read-only viewer wrote to the pane")
    }

    // Kampr's own surface comes first. A pane that keeps no ring can still be taller than the
    // window — or zoomed past it — and that scroll belongs here until it runs out.
    @Test
    fun theSurfaceIsSpentBeforeTheProgramIsGivenTheNotch() = runComposeUiTest {
        val io = AgentIo("claude")
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(rows = 90, caretRow = 6), session, io = io)
        assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")

        wheel(-1f)
        assertTrue(session.view.scrollY > 0f, "the wheel did not move Kampr's own surface")
        assertEquals(emptyList(), io.typed, "the notch went to the pane while the surface still had room")

        repeat(60) { wheel(-1f) }
        assertEquals(session.view.maxScroll, session.view.scrollY, "the surface never reached its end")
        assertTrue(io.typed.isNotEmpty(), "the surface ran out and the notch went nowhere")
        assertTrue(io.typed.all { it.startsWith(WHEEL_UP) }, "a spent surface sent something other than wheel-up")
    }
}
