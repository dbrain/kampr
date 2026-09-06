package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
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

// The operator, on 0.1.67, in a browser on the desk: *"had nvtop open, closed it — bounced —
// scrolled back down manually"*, three times over one history search. Where it bounced to: *"up
// into older output, roughly a screenful"*.
//
// The measured half is what a full-screen program does to the pane on its way out, which probe
// #499 named as the one shape it had not covered: *"the same exit on a pane whose shell was near
// the top of the grid, which is the shape that moves the floor by a screenful"*. Probe #502 is
// that shape. Off `herdr terminal session observe` on a real pane, the frames of one alt-screen
// cycle on a pane whose shell had just cleared:
//
//   * with the program up: **every row written, caret at row 0, `visible=false`** — the record
//     ends on the last row, so the record's floor is zero
//   * on the way out: **five rows written of forty, caret on row 4** — the record ends 36 rows
//     above the bottom of the grid, and the floor is 35 of them
//
// Nothing moved but the screen, and the floor swung by seven eighths of the pane. A surface that
// rests on that floor is hauled a screenful into history the moment the program exits, and pinned
// there: a hand's floor is the same number, so the operator cannot scroll back to their own
// terminal.
//
// The floor is right about blank tail and wrong about where to find the room for it. Its job is to
// keep the end of the record on the screen; what it did was scroll the pane's *own grid* off the
// top and fill the view with rows the operator had scrolled away from. On the desk, where the pane
// is the size of the view, there was nothing to fix in the first place — the whole grid is on the
// screen at the bottom of the surface, blank tail and all, which is what the machine itself draws.
private val DESK = 1600.dp to 900.dp
private val PHONE = 411.dp to 914.dp

private const val GRID_ROWS = 24
private const val SHELL_ROWS = 5
private const val RING = 300

private const val DEEP_ROWS = 200

private fun row(index: Int) = RowDiff(index, listOf(Run(0, "[03:04:31 dbrain@comingclean scratchpad]$ line $index")))

private fun blank(index: Int) = RowDiff(index, listOf(Run(0, " ")))

// A pane a full-screen program owns: every row of the grid written, and the caret parked on the
// first of them with the cursor hidden — nvtop's frame as the node delivered it (#502).
private fun ownedByTheProgram(rows: Int): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = rows,
            rowsData = (0 until rows).map(::row),
            cursor = Cursor(0, 0, false),
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
    return pane
}

// And the frame after it: the main screen back, the shell near the top of it, the rest blank tail.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.givesTheScreenBack(pane: PaneState, rows: Int) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = (0 until rows).map { if (it < SHELL_ROWS) row(it) else blank(it) },
            cursor = Cursor(0, SHELL_ROWS - 1, true),
            links = emptyList(),
        ),
    )
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.watching(
    size: Pair<Dp, Dp>,
    rows: Int,
): Pair<PaneState, PaneSession> {
    val pane = ownedByTheProgram(rows)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
    assertEquals(RING, pane.scrollback.historyRows, "the ring is what a hauled reader lands in")
    assertTrue(
        session.view.maxScroll > rows * session.grid.cellHeight,
        "there has to be more travel above this grid than the grid itself, or nothing is tested",
    )
    assertTrue(session.view.following, "the reader has touched nothing and must still be following")
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
class LeavingAFullScreenProgramTest {
    // The desk, where the pane is the size of the view: the whole grid is on the screen with the
    // surface at the bottom of it, so there is no room to be made and nothing to make it out of.
    @Test
    fun aProgramGivingTheScreenBackDoesNotHaulTheViewportIntoHistory() = runComposeUiTest {
        val (pane, session) = watching(DESK, GRID_ROWS)
        val view = session.view
        assertEquals(0f, view.scrollY, 0.01f, "the surface did not open on the bottom of the grid")
        assertTrue(
            onScreen(pane, session, 0) && onScreen(pane, session, GRID_ROWS - 1),
            "the grid has to fit the view here, or this is not the operator's pane",
        )

        givesTheScreenBack(pane, GRID_ROWS)

        assertEquals(
            0f,
            view.scrollY,
            0.01f,
            "closing the program carried the reader ${view.scrollY / session.grid.cellHeight} rows " +
                "up the pane, into history they had not asked for",
        )
        assertTrue(
            onScreen(pane, session, GRID_ROWS - 1),
            "the bottom of the pane's own grid is off the screen, so the view is not the terminal",
        )
    }

    // And the hand is not pinned above it either. The two floors are one number here, so a follower
    // hauled into history is a reader who cannot scroll back out of it.
    @Test
    fun theBottomOfTheGridIsStillWhereAHandMayGo() = runComposeUiTest {
        val (pane, session) = watching(DESK, GRID_ROWS)
        givesTheScreenBack(pane, GRID_ROWS)
        assertEquals(
            0f,
            session.view.contentFloor,
            0.01f,
            "a grid that fits the view has nowhere to travel, and the hand was given ${
                session.view.contentFloor / session.grid.cellHeight
            } rows of history to be stuck above",
        )
    }

    // The phone half of the same rule, on the pane the floor was written for: a grid far taller
    // than the view, with the record stopping near the top of it. The surface still climbs — the
    // end of the record has to be on the screen — but it stops at the top of the pane's own grid.
    // Past that it is spending the operator's history to hide the pane's blank tail.
    @Test
    fun theFloorNeverClimbsAboveTheTopOfThePanesOwnGrid() = runComposeUiTest {
        val (pane, session) = watching(PHONE, DEEP_ROWS)
        givesTheScreenBack(pane, DEEP_ROWS)
        val view = session.view
        val topOfTheGrid = view.maxScroll - RING * session.grid.cellHeight
        // Both halves, because a floor that gave up climbing would pass the bound below and put
        // the record a hundred and eighty rows under the fold.
        assertTrue(
            onScreen(pane, session, SHELL_ROWS - 1),
            "the end of the record is off the screen, which is the thing the floor is for",
        )
        assertTrue(view.contentFloor > 0f, "the floor stopped climbing at all on a grid this deep")
        assertTrue(
            view.contentFloor <= topOfTheGrid + 0.01f,
            "the floor is ${view.contentFloor}px, which is ${
                (view.contentFloor - topOfTheGrid) / session.grid.cellHeight
            } rows above the first row of the grid",
        )
    }
}
