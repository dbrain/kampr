package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class ClaimIo : PaneIo {
    val claims = mutableListOf<Pair<Int, Int>>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override suspend fun claimMatch(paneId: String, cols: Int, rows: Int, grow: Boolean): Boolean {
        claims += cols to rows
        return true
    }
}

private fun written(cols: Int, rows: Int = 24): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    val lines = (0 until rows).map { "$ line $it" }
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = cols,
            rows = rows,
            rowsData = lines.mapIndexed { index, text -> RowDiff(index, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, rows - 1, true),
            links = emptyList(),
        ),
    )
    return pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.claimOn(pane: PaneState): Pair<Int, Int> {
    val io = ClaimIo()
    phoneTerminal(pane, PaneSession(Phone.PANE), width = 1624.dp, height = 1000.dp, io = io)
    waitUntil(timeoutMillis = 3_000) { io.claims.isNotEmpty() }
    return io.claims.last()
}

@OptIn(ExperimentalTestApi::class)
class ColumnBarTest {
    @Test
    fun aPaneThatFitsItsViewKeepsNoBarUnderIt() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session, width = 1280.dp, height = 800.dp)
        assertTrue(
            onAllNodes(columnBar).fetchSemanticsNodes().isEmpty(),
            "every column and the live edge are on screen, so there is no position to report",
        )
        assertEquals(0f, session.indicatorHeight, "the grid is still reserving room for a bar that says nothing")
    }

    @Test
    fun aPaneWiderThanItsViewSaysWhichColumnsAreShowing() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session, io = ReadableIo)
        assertEquals(1, onAllNodes(columnBar).fetchSemanticsNodes().size, "columns are off screen and nothing says so")
        assertEquals(
            0f,
            session.indicatorHeight,
            "the bar took room from the grid, so its coming and going reshapes the view it reports on",
        )
    }

    // The operator: "the horizontal scroll bar ... covers the bottom part of the terminal". A
    // grid written to its last row with the caret on it, on a phone that cannot show every column:
    // at the live edge the row being typed into has to rest above the bar, not behind it.
    @Test
    fun theLastRowAndTheCaretRestAboveTheBarAtTheLiveEdge() = runComposeUiTest {
        val pane = written(cols = 200)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, io = ReadableIo)
        waitForIdle()
        assertEquals(1, onAllNodes(columnBar).fetchSemanticsNodes().size, "columns are off screen and nothing says so")
        val last = pane.cells.rows - 1
        assertTrue(
            onScreen(pane, session, last),
            "the last row ends at ${rowBottom(pane, session, last)}, behind a bar starting at ${visibleBottom()}",
        )
        assertTrue(
            onScreen(pane, session, pane.cursor.row),
            "the caret's row ends at ${rowBottom(pane, session, pane.cursor.row)}, behind a bar at ${visibleBottom()}",
        )
    }

    // The operator, after the gap went in: "should be full width and flush with the view below it,
    // the terminal area shouldnt draw under it at all". So the bar is a strip across the whole
    // view standing on the chrome under it, and the grid runs down to its top edge and no further.
    @Test
    fun theBarIsAStripAcrossTheViewStandingOnTheChromeWithTheGridDownToIt() = runComposeUiTest {
        val pane = written(cols = 200)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, io = ReadableIo)
        waitForIdle()
        val bar = onAllNodes(columnBar).fetchSemanticsNodes().single().boundsInRoot
        val (left, right, top, bottom) = with(density) {
            listOf(bar.left.toDp(), bar.right.toDp(), bar.top.toDp(), bar.bottom.toDp())
        }
        assertEquals(0f, left.value, 0.5f, "the bar starts ${left} in from the view's left edge")
        assertEquals(411f, right.value, 0.5f, "the bar stops at ${right} of a 411 dp view")
        assertEquals(stripTop().value, bottom.value, 0.5f, "the bar floats above the chrome under it")
        val last = rowBottom(pane, session, pane.cells.rows - 1)
        assertEquals(top.value, last.value, 0.5f, "the last row ends at $last, not at the bar's top edge $top")
    }

    // The room the bar makes is travel, never fit: a bar that took rows off what the view says it
    // can show would resize a held pane each time columns went off screen and back.
    @Test
    fun theBarDoesNotChangeTheSizeTheViewAsksFor() {
        var fits: Pair<Int, Int>? = null
        var clipped: Pair<Int, Int>? = null
        runComposeUiTest {
            fits = claimOn(written(cols = 40))
            assertTrue(onAllNodes(columnBar).fetchSemanticsNodes().isEmpty(), "a pane that fits showed a bar")
        }
        runComposeUiTest {
            clipped = claimOn(written(cols = 400))
            assertEquals(1, onAllNodes(columnBar).fetchSemanticsNodes().size, "a pane wider than a desk showed no bar")
        }
        assertEquals(fits, clipped, "the bar changed the grid the desk asks the pane for")
    }
}
