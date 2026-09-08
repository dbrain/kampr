package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The operator, on a phone, having pressed up for an old command and Enter to run it and touched
// nothing else: *"i started with the end of the terminal visible on screen, didn't scroll and it
// never stayed with the live edge"*. Their screenshot's strip read **448 rows back**, and 448 was
// the depth of that pane's ring — `historyRows`, which is what the strip had been handed since it
// was written. So it says "rows back" whenever a pane has *any* history, tells a reader sitting on
// the live edge they are hundreds of rows behind it, and says the same to a screen reader. Both the
// operator and I built a diagnosis on that number before reading where it came from.
private const val RING = 448
private const val GRID_ROWS = 33
private const val NOTCHES = 4

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aPaneWithARing(): PaneSession {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = GRID_ROWS,
            rowsData = (0 until GRID_ROWS).map { RowDiff(it, listOf(Run(0, "output line $it"))) },
            cursor = Cursor(0, GRID_ROWS - 1, true),
            links = emptyList(),
        ),
    )
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = Phone.PANE,
            fromTop = 0,
            rows = (0 until RING).map { RowDiff(it, listOf(Run(0, "history row $it"))) },
            totalRows = RING,
            complete = true,
            capped = false,
        ),
    )
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = 1600.dp, height = 900.dp)
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
    return session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.saying(text: String): Int =
    onAllNodesWithContentDescription(text, substring = true).fetchSemanticsNodes().size

@OptIn(ExperimentalTestApi::class)
class RowsBackTest {
    @Test
    fun aReaderOnTheLiveEdgeIsNotToldTheyAreBehindIt() = runComposeUiTest {
        val session = aPaneWithARing()
        assertTrue(session.view.following, "the reader has touched nothing, or nothing is tested")
        assertTrue(
            session.view.maxScroll > 0f,
            "the pane has to hold a ring, or the strip has nothing to get wrong",
        )
        assertEquals(
            0,
            saying("rows back"),
            "a reader on the live edge was told they were $RING rows behind it",
        )
    }

    // And when they really are behind it, the number is the distance rather than the ring.
    @Test
    fun theStripCountsTheRowsBehindRatherThanTheRowsHeld() = runComposeUiTest {
        val session = aPaneWithARing()
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            repeat(NOTCHES) { scroll(-1f, ScrollWheel.Vertical) }
        }
        waitForIdle()

        val travelled = ((session.view.scrollY - session.view.band.floor) / session.grid.cellHeight)
            .toInt()
        assertTrue(
            travelled in 1 until RING,
            "the wheel went $travelled rows, which is not short of the ring, so nothing is tested",
        )
        assertEquals(1, saying("$travelled rows back"), "the strip does not say how far back it is")
        assertEquals(0, saying("$RING rows back"), "the strip is still reporting the ring's depth")
    }
}

// The operator, on a new terminal, running `df -h` until it filled the screen: *"when full wait
// maybe 2s then it's consistently jumping up to the last `df -h` entry"*. Nothing scrolled — the
// **cell** changed. The opening zoom used to be a fill over the live grid *plus the ring*, so the
// same pane answered 1.067x with no history and 0.560x once the node's first scrollback frame
// landed about two seconds later, and the text halved under a reader who had chosen nothing.
//
// The operator on the fill itself, which produced both ends of its range: *"sometimes being zoomed
// at 0.4x which is ridiculously small, other times 3.7x which is ridiculously large. 1.0x seems to
// be the best on all devices."* A constant cannot be derived a second time, so there is nothing
// left to re-derive and nothing to freeze.
@OptIn(ExperimentalTestApi::class)
class OpeningZoomTest {
    @Test
    fun aNewTerminalOpensAtOneToOneAndTheRingArrivingDoesNotResizeIt() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE,
                cols = 94,
                rows = 40,
                rowsData = (0 until 40).map { RowDiff(it, listOf(Run(0, "/dev/nvme0n1p2 1.8T 64% /"))) },
                cursor = Cursor(27, 39, true),
                links = emptyList(),
            ),
        )
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, width = 411.dp, height = 914.dp)
        mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
        waitForIdle()
        assertEquals(1f, session.view.zoom, 0.001f, "a pane opens at 1.0x")
        val cell = session.grid.cellHeight

        // The node's first scrollback frame, the one that used to halve the cell.
        pane.applyPatch(
            ServerMsg.GridPatch(
                pane = Phone.PANE,
                rows = listOf(RowDiff(39, listOf(Run(0, "[dbrain@giftofthemagi2 ~]$ ")))),
                cursor = Cursor(27, 39, true),
                links = emptyList(),
            ),
        )
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = Phone.PANE,
                fromTop = 0,
                rows = (0 until 52).map { RowDiff(it, listOf(Run(0, "scrolled off $it"))) },
                totalRows = 52,
                complete = true,
                capped = false,
            ),
        )
        mainClock.advanceTimeBy(CARET_SETTLE_MS * 4)
        waitForIdle()

        assertEquals(1f, session.view.zoom, 0.001f, "the ring arriving re-derived the zoom")
        assertEquals(
            cell,
            session.grid.cellHeight,
            0.001f,
            "the cell changed size under a reader who had chosen nothing",
        )
    }
}
