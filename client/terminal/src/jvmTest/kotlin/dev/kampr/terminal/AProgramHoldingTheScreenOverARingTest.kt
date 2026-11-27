package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PaneModes
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.WHEEL_ROWS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// Short enough to fit the desk, so the program's whole screen is on show and no travel is Kampr's.
private const val GRID_ROWS = 30
private const val RING = 200

private class ProgramIo(private val agent: String?, private val cmd: String?, private val modes: PaneModes?) : PaneIo {
    val typed = mutableListOf<String>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) typed += msg.text
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = PaneInfo(paneId, "node", agent = agent, cmd = cmd, cols = 94, rows = GRID_ROWS, modes = modes)
}

private fun ring(fromTop: Int = 0, rows: Int = RING) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = fromTop,
    rows = (fromTop until fromTop + rows).map { RowDiff(it, listOf(Run(0, "shell era $it"))) },
    totalRows = rows,
    complete = true,
    capped = false,
)

// The shell session from before the program started, still held: neither herdr (which never labels
// muffit) nor a native node (whose harness list does not name it) supersedes it, so the ring sits
// above a program that owns every row of the grid.
private fun screenOverARing(): PaneState = Phone.filled(rows = GRID_ROWS, caretRow = GRID_ROWS - 1).also { it.applyScrollback(ring()) }

// What muffit-tui sets at startup, measured off a pty: `?1049h ?1000h ?1002h ?1003h ?1015h ?1006h`.
private val MUFFIT_MODES = PaneModes(mouse = "any", mouseSgr = true, altScreen = true)

private class Case(val label: String, val agent: String?, val cmd: String?, val modes: PaneModes?, val sent: String)

private val TAKERS = listOf(
    Case("muffit in a herdr pane, which reports no modes", "muffit", "muffit", null, "\u001b[<64;"),
    Case("muffit's TUI in a herdr pane, which labels it nothing (#699)", null, "muffit-tui", null, "\u001b[<64;"),
    Case("muffit in a native session, with its measured modes", "muffit", "muffit", MUFFIT_MODES, "\u001b[<64;"),
    Case("a program on the main screen that took the mouse", null, "htop", PaneModes(mouse = "any", mouseSgr = true), "\u001b[<64;"),
    Case("a pager on the alternate screen with alternate scroll", null, "less", PaneModes(altScroll = true, altScreen = true), "\u001bOA"),
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheelUp(notches: Int) {
    repeat(notches) {
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            scroll(-1f)
        }
        waitForIdle()
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragDown() {
    onRoot().performTouchInput {
        var y = 100f
        down(Offset(width / 2f, y))
        repeat(10) {
            y += 150f
            moveTo(Offset(width / 2f, y))
        }
        up()
    }
    waitForIdle()
}

private fun reportRow(report: String): Int = report.substringAfterLast(';').dropLast(1).toInt()

// The report, on 0.1.153 in a desktop browser: *"now that it actually scrolls, scrolling back isn't
// scrolling the TUI, it's taking me above the TUI rendering"*. muffit's TUI holds the alternate
// screen and asked for the mouse, so the wheel is its, as it is in every terminal (docs/26, rule 1).
// The ring above it is a shell nobody is looking at, and it was what the wheel moved.
@OptIn(ExperimentalTestApi::class)
class AProgramHoldingTheScreenOverARingTest {
    @Test
    fun aWheelOverAProgramThatTookItScrollsTheProgramAndNotTheRing() = runComposeUiTest {
        for (case in TAKERS) {
            val io = ProgramIo(case.agent, case.cmd, case.modes)
            val pane = screenOverARing()
            val session = PaneSession(Phone.PANE)
            deskTerminal(pane, session, io)
            waitForIdle()
            assertEquals(RING, pane.scrollback.historyRows, "${case.label}: the ring has to be held, or nothing is tested")
            val before = session.view.scrollY

            wheelUp(6)
            assertEquals(before, session.view.scrollY, 0.01f, "${case.label}: the wheel scrolled Kampr's ring")
            assertTrue(io.typed.isNotEmpty(), "${case.label}: the program was sent nothing")
            assertTrue(io.typed.all { it.startsWith(case.sent) }, "${case.label}: ${io.typed.first().drop(1)}")
            if (case.sent.startsWith("\u001b[<")) {
                assertTrue(
                    io.typed.all { reportRow(it) in 1..GRID_ROWS },
                    "${case.label}: a report named a row of the ring, not of the program's screen: ${io.typed.first().drop(1)}",
                )
            }
        }
    }

    @Test
    fun aFingerOverMuffitScrollsMuffitAndNotTheRing() = runComposeUiTest {
        val io = ProgramIo("muffit", "muffit", MUFFIT_MODES)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(screenOverARing(), session, io = io)
        waitForIdle()
        val top = (session.view.maxScroll - RING * session.grid.cellHeight).coerceAtLeast(0f)

        dragDown()
        assertTrue(session.view.scrollY <= top + 0.01f, "the drag went into the ring: ${session.view.scrollY} past $top")
        assertTrue(io.typed.isNotEmpty(), "the drag never reached muffit")
        assertTrue(io.typed.all { reportRow(it) in 1..GRID_ROWS }, "a report named a row of the ring: ${io.typed.first().drop(1)}")
    }

    // A harness the node does supersede leaves the travel of the ring it took away (`reserved`, so
    // the surface does not jump when the rows come back). Nothing is drawn there, and a wheel spent
    // its way up through that blank before the first notch reached the program.
    @Test
    fun aWheelOverAHarnessIsNotSpentOnTheRoomLeftForASupersededRing() = runComposeUiTest {
        val io = ProgramIo("claude", "claude", null)
        val pane = screenOverARing()
        val session = PaneSession(Phone.PANE)
        deskTerminal(pane, session, io)
        waitForIdle()
        pane.applyScrollback(ServerMsg.Scrollback(Phone.PANE, fromTop = RING, rows = emptyList(), totalRows = 0, complete = true, capped = false))
        waitForIdle()
        assertEquals(0, pane.scrollback.historyRows, "the ring has to be superseded, or nothing is tested")
        val before = session.view.scrollY

        wheelUp(3)
        assertEquals(before, session.view.scrollY, 0.01f, "the wheel travelled into the blank left for the ring")
        assertEquals(3 * WHEEL_ROWS.toInt(), io.typed.size, "a report a row, for every notch: ${io.typed}")
    }
}
