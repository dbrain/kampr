package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
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

// The operator, using Kampr instead of the terminal it is in front of: *"i keep hitting issues like
// this where things are just jumping around constantly"*. On `w24:p1` of the hub, watched live
// while they reproduced it (probe #504): an `adb pair` flow, the pane held at the view's own 289x69
// for every second of it, and the ring going **355 → 442** as the command scrolled.
//
// They had not scrolled. What made them a parked reader was a wheel notch from an earlier session
// with the pane — `following` is cleared by one notch and nothing but typing sets it again, so it
// outlives leaving the pane, the conversation view, and the hour in between. Replayed at those
// numbers, three rows became **ninety**: one notch off the live edge, and 87 rows of output carried
// them 87 rows further from it, because `carryHistory`'s exemption is `scrollY <= floor + 0.5f` and
// on a pane matched to the view the floor is zero. Half a pixel of forgiveness.
//
// **The carry is for a reader of history and this reader is not one.** Its whole purpose is to hold
// the rows under an eye still while the surface grows underneath them, and that is right for
// somebody who has gone back to read. Somebody sitting at the bottom of the pane watching a command
// run is not reading anything — holding them still is what walks them off the thing they are
// watching, one batch of output at a time, with nothing but a keystroke to bring them back.
//
// The line is a screenful. Inside it the live edge is one gesture away and the reader is watching
// the pane; beyond it they are in its history and anchored to it, exactly as before.
private val DESK = 1600.dp to 900.dp

private const val GRID_ROWS = 33
private const val RING = 355

// The four deliveries the node made while the operator watched, totalling the 87 rows the hub's own
// `max_offset_from_bottom` moved by.
private val BATCHES = listOf(39, 21, 4, 23)
private const val SCROLLED_OFF = 87

private fun history(from: Int, count: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = 0,
    rows = (from until from + count).map { RowDiff(it, listOf(Run(0, "scrolled off $it"))) },
    totalRows = from + count,
    complete = true,
    capped = false,
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aPaneMatchedToTheView(): Pair<PaneState, PaneSession> {
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
    pane.applyScrollback(history(0, RING))
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = DESK.first, height = DESK.second)
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
    assertEquals(
        0f,
        session.view.band.floor,
        0.01f,
        "the pane has to be matched to the view, or this is not the operator's pane",
    )
    return pane to session
}

// Rows only enter the ring because the pane wrote lines, and the write is what this surface
// recomposes on — the ring is not snapshot state. A scrollback frame arrives behind the grid frame
// that produced it, the way the node sends one.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.theCommandScrolls(pane: PaneState) {
    var held = RING
    for (batch in BATCHES) {
        pane.applyPatch(
            ServerMsg.GridPatch(
                pane = Phone.PANE,
                rows = listOf(RowDiff(GRID_ROWS - 1, listOf(Run(0, "receiving objects $held")))),
                cursor = Cursor(held % 40, GRID_ROWS - 1, true),
                links = emptyList(),
            ),
        )
        pane.applyScrollback(history(held, batch))
        held += batch
        mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
        waitForIdle()
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheelUp(notches: Int) {
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        repeat(notches) { scroll(-1f, ScrollWheel.Vertical) }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class StayingWithTheOutputTest {
    @Test
    fun aReaderAtTheBottomIsNotCarriedOffTheOutputTheyAreWatching() = runComposeUiTest {
        val (pane, session) = aPaneMatchedToTheView()
        val view = session.view
        val cell = session.grid.cellHeight

        wheelUp(1)
        val parked = view.scrollY
        assertTrue(parked > 0f, "the notch went nowhere, so nothing is tested")
        assertTrue(!view.following, "one notch has to clear following, or this is a different defect")

        theCommandScrolls(pane)

        assertEquals(
            parked,
            view.scrollY,
            0.01f,
            "$SCROLLED_OFF rows of output carried a reader ${parked / cell} rows off the live edge " +
                "to ${view.scrollY / cell} rows — the pane walked out from under them",
        )
    }

    // The other half, and the reason the carry exists: a reader who has genuinely gone back into
    // the history is held to their own rows, so the surface growing underneath does not slide the
    // lines they are reading up the screen.
    @Test
    fun aReaderUpInTheHistoryIsStillHeldToTheirRows() = runComposeUiTest {
        val (pane, session) = aPaneMatchedToTheView()
        val view = session.view
        val cell = session.grid.cellHeight

        wheelUp(30)
        val parked = view.scrollY
        assertTrue(
            parked > session.view.maxScroll.coerceAtMost(900f) * 0.0f + 30f * cell - 0.5f,
            "the wheel has to have travelled a screenful and more, or this is the other case",
        )

        theCommandScrolls(pane)

        assertEquals(
            parked + SCROLLED_OFF * cell,
            view.scrollY,
            0.01f,
            "a reader ${parked / cell} rows up was not carried by the $SCROLLED_OFF rows that " +
                "entered the ring beneath them, so their own rows slid up the screen",
        )
    }
}
