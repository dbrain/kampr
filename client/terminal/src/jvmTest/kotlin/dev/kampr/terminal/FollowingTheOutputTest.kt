package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTextInput
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

// The other half of a full-screen redraw, and the one the *content* floor is a function of: the
// rows themselves go away and come back. Blanking a row is a patch with no runs in it, which is
// how the node ships one — `CellBuffer.apply` fills whatever the runs did not cover.
//
// The caret is left wherever the caller put it, because that is where an agent's TUI leaves it:
// the input box holds still while the block above or below it is repainted, and a caret that moved
// with every row would hide the whole defect behind its own settled reading.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.rewrite(
    pane: PaneState,
    session: PaneSession,
    rows: IntProgression,
    caretRow: Int,
    text: (Int) -> String?,
): List<Float> = rows.map { row ->
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, text(row)?.let { listOf(Run(0, it)) } ?: emptyList())),
            cursor = Cursor(0, caretRow, true),
            links = emptyList(),
        ),
    )
    waitForIdle()
    session.view.scrollY
}

// A grid written to its last row, with the caret parked a screenful above the end of it — which is
// what makes the *content* floor the one holding the surface up rather than the caret's.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.redrawn(size: Pair<Dp, Dp>, caretRow: Int): Pair<PaneState, PaneSession> {
    val pane = Phone.filled(rows = DEEP_ROWS, caretRow = caretRow)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    assertEquals(0f, session.view.contentFloor, "this pane is written to its last row")
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

    // The same rule asked of the second floor. The caret holds still in its input box while the
    // block *below* it is blanked and rewritten — an agent's TUI, `docker compose pull`'s block one
    // row further down, any full-screen repaint that does not walk its own caret — so the caret's
    // settled reading says nothing about it and the record's end is what moves: fifty rows up the
    // surface as the block goes, and fifty back as it returns. A floor that followed the record
    // frame by frame would drag a following viewport up and drop it back, which is #428's flash
    // arriving by the other of the two floors.
    //
    // Asserted at every step of both halves, for #433's reason: a sweep read only at its end passes
    // while the surface oscillates the whole way through it.
    @Test
    fun aRedrawThatBlanksTheBlockBelowTheCaretAndRewritesItMovesNothing() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val caret = 150
                val (pane, session) = redrawn(size, caretRow = caret)
                val resting = session.view.scrollY
                val block = caret + 1..DEEP_ROWS - 1

                assertEquals(
                    block.map { resting },
                    rewrite(pane, session, block.reversed(), caret) { null },
                    "$size: blanking the block took the surface off the row it was resting on",
                )
                assertEquals(
                    block.map { resting },
                    rewrite(pane, session, block, caret) { "  redrawn row $it" },
                    "$size: rewriting the block took the surface off the row it was resting on",
                )

                mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                waitForIdle()
                assertEquals(
                    resting,
                    session.view.scrollY,
                    "$size: the redraw put the record back exactly where it was and the surface " +
                        "moved anyway once the readings settled",
                )
            }
        }
    }

    // And what the settled reading is *for*, which is the same thing every floor here is for: a
    // record that really has got shorter takes the surface with it. `clear` is the whole of the
    // case — a herdr pane keeps the height the desk gave it, so a cleared shell is three rows of
    // record and a hundred and ninety-seven of nothing, and a viewport left where the record used
    // to end is a screenful of blank with the prompt off the top of it.
    @Test
    fun aClearedShellIsNotLeftStaringAtTheRowsItCleared() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (pane, session) = deep(size, caretRow = 120)
                rewrite(pane, session, 3..DEEP_ROWS - 1, caretRow = 2) { null }
                rewrite(pane, session, 2..2, caretRow = 2) { "[20:36:31 dbrain@comingclean kampr]$ " }
                mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                waitForIdle()

                for (row in 0..2) {
                    assertTrue(
                        onScreen(pane, session, row),
                        "$size: row $row of a three-row record is off the screen at " +
                            "${rowTop(pane, session, row)} — the surface stayed in the tail",
                    )
                }
            }
        }
    }

    // The two settled readings run on two clocks — the caret's restarts when the caret moves, the
    // record's when anything at all is written — so there is a window in which one of them is new
    // and the other is a rectangle old. In it the record can appear to end *above* the caret, which
    // is a shape neither floor has an answer for: the band inverts, its ceiling is dragged up to
    // meet the floor, and the surface parks a screenful above the row being typed into.
    //
    // The caret is content wherever it is, and taking the nearer of the two readings is what says
    // so in the only frame where they disagree.
    @Test
    fun aCaretThatSettlesBeforeTheRecordIsStillOnTheScreenInBetween() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val parked = DEEP_ROWS - 10
                val (pane, session) = deep(size, caretRow = 3)
                rewrite(pane, session, 3..3, caretRow = parked) { "[20:36:31 dbrain@comingclean]$ " }
                mainClock.advanceTimeBy(CARET_SETTLE_MS / 2)

                // Anything at all arriving restarts the record's reading and leaves the caret's
                // where it was — which is the whole of the window this test is about.
                rewrite(pane, session, 1..1, caretRow = parked) { "  a line somewhere else" }
                // To exactly the moment the caret's reading lands and no further: the record's is
                // half an interval behind it and must not be allowed to arrive as well, or the
                // window closes before anything has been asked of it.
                mainClock.advanceTimeBy(CARET_SETTLE_MS / 2)
                waitForIdle()
                assertTrue(
                    onScreen(pane, session, parked),
                    "$size: the caret settled on row $parked and the surface parked at " +
                        "${session.view.scrollY} of ${session.view.maxScroll}, with the row it is " +
                        "on at ${rowTop(pane, session, parked)}",
                )

                mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                waitForIdle()
                assertTrue(
                    onScreen(pane, session, parked),
                    "$size: and it left the screen once the record's reading caught up",
                )
            }
        }
    }

    // The way back, and the only one there is: a hand that has parked is left where it parked by
    // every rule here, so nothing but typing re-arms `following` — which is the bargain #380 and
    // #428 both leaned on and neither had a test for. It matters more now, not less: a hand's floor
    // is the end of the record and a follower's is the higher of two, so there are panes where the
    // hand cannot land on the follower's resting place by travelling to it.
    @Test
    fun typingBringsAReaderInTheHistoryBackToTheLiveEdge() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (_, session) = deep(size, caretRow = 120)
                val resting = session.view.scrollY
                repeat(6) {
                    onRoot().performMouseInput {
                        moveTo(Offset(width / 2f, height / 2f))
                        scroll(-1f, ScrollWheel.Vertical)
                    }
                }
                waitForIdle()
                assertTrue(
                    session.view.scrollY > resting && !session.view.following,
                    "$size: the wheel did not take the reader up into the record and off the edge",
                )

                session.openKeyboard()
                waitForIdle()
                onNode(hasSetTextAction()).performTextInput("l")
                waitForIdle()

                assertTrue(session.view.following, "$size: typing did not re-arm the live edge")
                assertEquals(
                    resting,
                    session.view.scrollY,
                    "$size: typing re-armed following and left the viewport where the hand put it",
                )
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
