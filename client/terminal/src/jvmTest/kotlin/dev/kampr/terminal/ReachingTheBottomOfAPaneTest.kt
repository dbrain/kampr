package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipe
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The two sizes the report came from. "mobile is similarly zoomed in a lot", so this is not a
// desktop-only rule: what makes the bottom unreachable is a grid taller than the rectangle with the
// caret above the last row of it, and every phone against a herdr pane is that.
private val DESK = 1600.dp to 900.dp
private val PHONE = 411.dp to 914.dp

private const val DEEP_ROWS = 200

// Where the caret is parked on both panes below, and on the second of them the last row of the
// record as well. Far enough down that the grid still overflows underneath it by screenfuls.
private const val CARET = 120

// Enough notches to walk the whole of a 200-row surface at three rows a notch, twice over.
private const val NOTCHES_TO_THE_END = 200

// A full-screen redraw's pane: written to the last row of the grid, with the caret left in the
// middle where the program put it. The record runs *below* the caret, so the caret's floor is a
// whole screenful above the end of it — this is the pane #428 was about, and the bottom of its
// grid and the end of its record are the same row.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.written(size: Pair<Dp, Dp>): Pair<PaneState, PaneSession> {
    val pane = Phone.filled(rows = DEEP_ROWS, caretRow = CARET)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
    assertTrue(
        session.view.band.floor > 0f,
        "the caret has to be holding the surface off the bottom of the grid, or nothing is tested",
    )
    assertEquals(0f, session.view.contentFloor, 0.01f, "this pane is written to its last row")
    return pane to session
}

// A shell's pane: the same 200-row grid the desk made, with the record stopping at the caret and
// eighty rows of nothing under it. The bottom of the grid is not the bottom of anything.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tailed(size: Pair<Dp, Dp>): Pair<PaneState, PaneSession> {
    val pane = Phone.shell(rows = DEEP_ROWS, caretRow = CARET)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    val view = session.view
    assertEquals(
        (DEEP_ROWS - 1 - CARET) * session.grid.cellHeight,
        view.contentFloor,
        0.01f,
        "the floor has to be the last written row on the bottom of the rectangle",
    )
    assertTrue(
        view.contentFloor > 0f && view.contentFloor < view.maxScroll,
        "there has to be blank tail below the record (${view.contentFloor}) and travel above it " +
            "(${view.maxScroll}), or nothing is tested",
    )
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheelToTheBottom() {
    repeat(NOTCHES_TO_THE_END) {
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            scroll(1f, ScrollWheel.Vertical)
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragToTheBottom() {
    repeat(20) {
        onRoot().performTouchInput {
            swipe(
                start = Offset(centerX, centerY + 300f),
                end = Offset(centerX, centerY - 300f),
                durationMillis = 300,
            )
        }
        waitForIdle()
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.caretTo(pane: PaneState, row: Int) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, "$ line $row")))),
            cursor = Cursor(0, row, true),
            links = emptyList(),
        ),
    )
    waitForIdle()
}

// What "the bottom" is worth asserting about: the last row anybody wrote is on the screen, and the
// first row nobody did is under the fold. Together they say the surface stopped exactly on the end
// of the record — one of them alone passes for a viewport parked anywhere in the tail.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.landedOnTheRecord(pane: PaneState, session: PaneSession, last: Int) {
    assertTrue(
        onScreen(pane, session, last),
        "the last written row ($last) is off the screen: it sits at ${rowTop(pane, session, last)} " +
            "of a rectangle running ${Phone.HEADER}..${stripTop()}",
    )
    assertTrue(
        !onScreen(pane, session, last + 1),
        "row ${last + 1} is blank and on the screen below the record, at " +
            "${rowTop(pane, session, last + 1)} — the hand parked in the tail",
    )
}

// The report, verbatim: "the terminal pane on wasm desktop keeps bouncing around and landing back
// where i last typed instead of the bottom of the screen".
//
// The caret band's floor was wired into the drag's own clamp, and the floor is where a *follower*
// rests — the least scroll that leaves the caret on screen — not a limit on where a hand may go.
// So on any pane taller than the rectangle whose caret sits above the last row, everything below
// the caret's screenful was unreachable: the wheel stopped early, and the surface a hand did win
// was thrown back at the next frame that moved the caret, by `max(scrollY, floor)`.
//
// Both halves are the same rule. The band governs a viewport that is following and nothing else.
//
// **And the hand has a floor of its own**, which taking the band's away is what left it without:
// zero is the bottom of the *grid*, and a herdr pane is as tall as the desk made it. On a pane
// whose output stops above the last row — a shell after `clear`, four lines in a ninety-row window
// — a hand that may reach zero drags into blank tail and is given a screenful of nothing with no
// way to tell it from the pane having died. The end of the record is the end of the travel.
@OptIn(ExperimentalTestApi::class)
class ReachingTheBottomOfAPaneTest {
    @Test
    fun aWheelReachesTheEndOfTheRecordOnADesk() = runComposeUiTest {
        val (pane, session) = written(DESK)
        wheelToTheBottom()
        assertEquals(
            0f,
            session.view.scrollY,
            0.01f,
            "the wheel stopped ${session.view.scrollY}px above the end of a record written to the " +
                "last row of the grid, at the caret floor of ${session.view.band.floor}px",
        )
        landedOnTheRecord(pane, session, DEEP_ROWS - 1)
    }

    @Test
    fun aWheelReachesTheEndOfTheRecordOnAPhone() = runComposeUiTest {
        val (pane, session) = written(PHONE)
        wheelToTheBottom()
        assertEquals(0f, session.view.scrollY, 0.01f, "the wheel stopped short of the record's end")
        landedOnTheRecord(pane, session, DEEP_ROWS - 1)
    }

    @Test
    fun aDragReachesTheEndOfTheRecord() = runComposeUiTest {
        val (pane, session) = written(PHONE)
        dragToTheBottom()
        assertEquals(0f, session.view.scrollY, 0.01f, "the finger stopped short of the record's end")
        landedOnTheRecord(pane, session, DEEP_ROWS - 1)
    }

    // And stays there. A reader who went past the live edge to read the tail of a pane is parked
    // there deliberately; the caret moving is the pane's business and not a reason to move them.
    @Test
    fun aReaderAtTheBottomIsNotDraggedBackToTheCaret() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (pane, session) = written(size)
                wheelToTheBottom()
                assertEquals(0f, session.view.scrollY, 0.01f, "$size: never reached the bottom")
                assertTrue(!session.view.following, "$size: the bottom is not the live edge here")

                for (row in listOf(0, 90, 199, 3, 120)) {
                    caretTo(pane, row)
                    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                    waitForIdle()
                    assertEquals(
                        0f,
                        session.view.scrollY,
                        0.01f,
                        "$size: the caret settling on row $row took the reader off the bottom " +
                            "of the grid and back to ${session.view.scrollY}px",
                    )
                }
            }
        }
    }

    // The regression the reachable bottom bought, in the operator's words: "I guess I'll see if I
    // notice weird gaps". A hand that may travel to the bottom of the *grid* travels off the end of
    // the record on every pane the shell has not filled, and eighty blank rows on a screen are
    // indistinguishable from a pane that has stopped answering.
    @Test
    fun aWheelStopsAtTheEndOfTheRecordOnADesk() = runComposeUiTest {
        val (pane, session) = tailed(DESK)
        wheelToTheBottom()
        assertEquals(
            session.view.contentFloor,
            session.view.scrollY,
            0.01f,
            "the wheel ran ${session.view.scrollY}px past the end of the record into its tail",
        )
        landedOnTheRecord(pane, session, CARET)
    }

    @Test
    fun aWheelStopsAtTheEndOfTheRecordOnAPhone() = runComposeUiTest {
        val (pane, session) = tailed(PHONE)
        wheelToTheBottom()
        assertEquals(
            session.view.contentFloor,
            session.view.scrollY,
            0.01f,
            "the wheel ran ${session.view.scrollY}px past the end of the record into its tail",
        )
        landedOnTheRecord(pane, session, CARET)
    }

    @Test
    fun aDragStopsAtTheEndOfTheRecord() = runComposeUiTest {
        val (pane, session) = tailed(PHONE)
        dragToTheBottom()
        assertEquals(
            session.view.contentFloor,
            session.view.scrollY,
            0.01f,
            "the finger ran ${session.view.scrollY}px past the end of the record into its tail",
        )
        landedOnTheRecord(pane, session, CARET)
    }

    // The end of the record is *reached*, not merely stopped short of: the same pane dragged the
    // other way and back again lands on the last written row rather than a screenful above it.
    // Without this half a floor of `maxScroll` — never move at all — would pass every test above.
    @Test
    fun theEndOfTheRecordIsStillReachedFromInsideTheHistory() = runComposeUiTest {
        val (pane, session) = tailed(PHONE)
        repeat(NOTCHES_TO_THE_END) {
            onRoot().performMouseInput {
                moveTo(Offset(width / 2f, height / 2f))
                scroll(-1f, ScrollWheel.Vertical)
            }
        }
        waitForIdle()
        assertEquals(
            session.view.maxScroll,
            session.view.scrollY,
            0.01f,
            "the wheel never reached the top of the surface, so the trip back proves nothing",
        )

        wheelToTheBottom()
        landedOnTheRecord(pane, session, CARET)
    }

    // A grid the rectangle can hold has no travel in either direction, and neither floor may invent
    // any. This is the arithmetic that would divide by a zero surface or clamp to a negative one.
    //
    // On a desk, because that is where a short grid is genuinely short: the opening zoom fills the
    // taller axis of the *paint* rectangle, so the same eight rows on a phone are magnified past
    // the content rectangle by exactly the chrome and do have somewhere to go.
    @Test
    fun aGridShorterThanTheViewportDoesNotMoveAtAll() = runComposeUiTest {
        val pane = Phone.shell(rows = 8, caretRow = 3)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, width = DESK.first, height = DESK.second)
        assertEquals(0f, session.view.maxScroll, 0.01f, "an 8-row pane fits a desk with room over")
        assertEquals(0f, session.view.contentFloor, 0.01f, "a grid with no travel has no floor")

        dragToTheBottom()
        assertEquals(0f, session.view.scrollY, 0.01f, "the finger moved a surface that cannot move")
        assertTrue(onScreen(pane, session, 3), "the caret row left the screen")
    }

    // What "the end of the record" means when the program has moved the cursor somewhere nothing is
    // written — `tput cup`, a full-screen app placing its own input line, a shell drawing a prompt
    // it has not printed into yet. The caret is the end of the record wherever it is: a floor taken
    // from the last written row alone would put the row being typed into a hundred and eighty rows
    // below the only place a hand could reach.
    @Test
    fun aCaretParkedBelowTheLastWrittenRowIsItselfTheEndOfTheRecord() = runComposeUiTest {
        val parked = DEEP_ROWS - 10
        val pane = Phone.shell(rows = DEEP_ROWS, caretRow = 3)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)
        pane.applyPatch(
            ServerMsg.GridPatch(
                pane = Phone.PANE,
                rows = emptyList(),
                cursor = Cursor(0, parked, true),
                links = emptyList(),
            ),
        )
        mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
        waitForIdle()
        assertEquals(
            (DEEP_ROWS - 1 - parked) * session.grid.cellHeight,
            session.view.contentFloor,
            0.01f,
            "the floor stopped at the last written row and left the caret below it",
        )

        dragToTheBottom()
        assertEquals(
            session.view.contentFloor,
            session.view.scrollY,
            0.01f,
            "the finger ran past the caret into the tail below it",
        )
        landedOnTheRecord(pane, session, parked)
    }

    // The other end of the same arithmetic: a pane the desk made and nothing has written to. Its
    // whole record is the row the caret is on, so there is nowhere to travel and the top of the
    // grid is the only place to be — a hand that could leave it would be leaving for two hundred
    // rows of nothing.
    @Test
    fun aPaneWithNothingOnItButItsCaretCannotBeDraggedIntoTheTail() = runComposeUiTest {
        val pane = Phone.bare(rows = DEEP_ROWS)
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)
        assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
        assertEquals(
            session.view.maxScroll,
            session.view.contentFloor,
            0.01f,
            "the record ends above the surface's own travel, so the travel is spent",
        )

        dragToTheBottom()
        assertEquals(
            session.view.maxScroll,
            session.view.scrollY,
            0.01f,
            "the finger dragged an empty pane ${session.view.maxScroll - session.view.scrollY}px " +
                "into its own blank tail",
        )
        assertTrue(onScreen(pane, session, 0), "the caret row left the screen")
    }
}
