package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val FIRST = 2
private const val LAST = 9
private val BLOCK = FIRST..LAST

// A grid far taller than either rectangle, which is what "zoomed in a lot" produces on both. The
// eight-row block of #380 never exceeded the band's own width, so the whole class of defect below
// was invisible to it.
private const val DEEP_ROWS = 200

private val DESK = 1600.dp to 900.dp
private val PHONE = 411.dp to 914.dp

private fun patch(rows: List<RowDiff>, cursorRow: Int) = ServerMsg.GridPatch(
    pane = Phone.PANE,
    rows = rows,
    cursor = Cursor(0, cursorRow, true),
    links = emptyList(),
)

// `docker compose pull` on a pane nobody has scrolled: a command line, then a block of one line
// per service that is redrawn in place — the caret walks to the top of the block, rewrites every
// line of it, and returns to the bottom. Every one of those is a frame the node ships.
private fun pulling(): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = 40,
            rowsData = listOf(
                RowDiff(0, listOf(Run(0, "[20:36:31 dbrain@comingclean deploy]$ docker compose pull"))),
                RowDiff(1, listOf(Run(0, "[+] Pulling 8/8"))),
            ),
            cursor = Cursor(0, FIRST, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun redraw(percent: Int) = BLOCK.map { row ->
    RowDiff(row, listOf(Run(0, " service-${row - FIRST}  Downloading [===>    ] $percent%")))
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.offScreen(pane: PaneState, session: PaneSession): List<Int> =
    BLOCK.filterNot { onScreen(pane, session, it) }

// A full-screen repaint, as an agent's TUI ships one: the caret goes home, every row is rewritten
// under it, and it returns to the input box. Each step is a frame, and the whole sweep is over
// long before anything has stopped anywhere.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sweepCaret(
    pane: PaneState,
    session: PaneSession,
    rows: List<Int>,
): List<Float> = rows.map { row ->
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, "  redrawn row $row")))),
            cursor = Cursor(0, row, true),
            links = emptyList(),
        ),
    )
    waitForIdle()
    session.view.scrollY
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.deep(size: Pair<Dp, Dp>, caretRow: Int): Pair<PaneState, PaneSession> {
    val pane = Phone.shell(rows = DEEP_ROWS, caretRow = caretRow)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    assertTrue(
        session.view.maxScroll > DEEP_ROWS * session.grid.cellHeight * 0.5f,
        "the grid has to be far taller than the rectangle, or the excursion fits the band and " +
            "nothing is tested",
    )
    return pane to session
}

// The report, verbatim: "when running commands that output some lines and update in place like a
// docker pull && docker up -d the wasm app (and maybe others) scroll up/don't track the output …
// when done sometimes i need to scroll up and back down for it to show me the completed terminal
// text entry".
//
// The surface rested *exactly* on the caret floor, and the floor is a minimum, not a place. So
// every frame that moved the caret moved the surface with it, in both directions and by the whole
// distance — and an in-place redraw moves the caret to the top of its block and back several times
// a second. The block being redrawn is what the operator is reading; the caret's excursion to the
// top of it is a rendering artefact of the command, not somewhere anybody asked to look.
@OptIn(ExperimentalTestApi::class)
class FollowingTheOutputTest {
    @Test
    fun anInPlaceRedrawKeepsTheBlockItIsRedrawingOnScreen() {
        runComposeUiTest {
            val pane = pulling()
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")

            pane.applyPatch(patch(redraw(0), cursorRow = LAST + 1))
            waitForIdle()
            assertTrue(
                offScreen(pane, session).isEmpty(),
                "rows ${offScreen(pane, session)} of the block were off screen the moment it was drawn",
            )

            for (percent in listOf(30, 60, 90)) {
                pane.applyPatch(patch(redraw(percent), cursorRow = FIRST))
                waitForIdle()
                assertTrue(
                    offScreen(pane, session).isEmpty(),
                    "$percent%: the caret went to the top of the block and took rows " +
                        "${offScreen(pane, session)} of it off the screen",
                )
                pane.applyPatch(patch(emptyList(), cursorRow = LAST + 1))
                waitForIdle()
                assertTrue(
                    offScreen(pane, session).isEmpty(),
                    "$percent%: the caret came back to the bottom of the block and took rows " +
                        "${offScreen(pane, session)} of it off the screen",
                )
            }
        }
    }

    @Test
    fun aCaretThatLeavesTheBlockAndComesStraightBackDoesNotMoveTheSurface() {
        runComposeUiTest {
            val pane = pulling()
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            pane.applyPatch(patch(redraw(0), cursorRow = LAST + 1))
            waitForIdle()
            val resting = session.view.scrollY

            for (percent in listOf(30, 60, 90)) {
                pane.applyPatch(patch(redraw(percent), cursorRow = FIRST))
                waitForIdle()
                assertTrue(
                    abs(session.view.scrollY - resting) < 0.5f,
                    "$percent%: the caret stepping up to the top of the block moved the surface " +
                        "from $resting to ${session.view.scrollY}",
                )
                pane.applyPatch(patch(emptyList(), cursorRow = LAST + 1))
                waitForIdle()
                assertTrue(
                    abs(session.view.scrollY - resting) < 0.5f,
                    "$percent%: the caret stepping back down moved the surface from $resting " +
                        "to ${session.view.scrollY}",
                )
            }
        }
    }

    // The other half of the report: "sometimes it flashes and sort of scrolls up then down".
    //
    // #380's block is eight rows on a 40-row pane, and the band is as wide as the rectangle — so
    // that excursion never left it and the surface never moved. Zoom in until the grid is three or
    // four screens tall and the caret's round trip is *wider than the band*, and the band, which
    // translates with the caret one pixel per pixel, drags the viewport up and then straight back
    // down again, once per repaint. Nothing about that is somebody asking to look anywhere.
    @Test
    fun aRepaintThatSweepsTheCaretAcrossATallGridMovesNothing() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (pane, session) = deep(size, caretRow = 120)
                val resting = session.view.scrollY
                val steps = listOf(0, 40, 90, 150, 199, 120)
                repeat(3) {
                    // Every step of the sweep, not only where it ends: a viewport dragged up the
                    // grid and back down again finishes exactly where it started, and that is the
                    // flash the operator reported rather than the absence of one.
                    assertEquals(
                        steps.map { resting },
                        sweepCaret(pane, session, steps),
                        "$size: the caret's round trip across the grid took the surface with it",
                    )
                }
            }
        }
    }

    // And the guarantee #175 and #380 bought, which has to survive all of that: a caret that
    // genuinely stops somewhere off screen still brings the viewport to it, by the least distance
    // that puts it back on — which is the floor when it went off the top. Without this the
    // operator types blind, which is the defect the band exists to prevent.
    @Test
    fun aCaretThatSettlesOffTheTopIsFetchedBackByTheLeastItCan() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (pane, session) = deep(size, caretRow = 150)
                val resting = session.view.scrollY
                sweepCaret(pane, session, listOf(10))
                assertEquals(
                    resting,
                    session.view.scrollY,
                    "$size: the surface moved before the caret had stopped anywhere",
                )

                mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                waitForIdle()
                assertTrue(
                    onScreen(pane, session, 10),
                    "$size: the caret settled on row 10 and the operator was left typing into a " +
                        "row off the top of the surface",
                )
                assertEquals(
                    session.view.band.floor,
                    session.view.scrollY,
                    "$size: the surface was moved past the least that puts the caret back on screen",
                )
            }
        }
    }
}
