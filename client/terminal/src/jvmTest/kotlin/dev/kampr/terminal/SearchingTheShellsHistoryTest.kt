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
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The operator, on 0.1.57: *"doing things like pressing up to see atuin history and launching
// docker commands has the terminal scroll up and I need to manually scroll down"*.
//
// The half that had to be measured rather than reasoned about is what atuin does to the pane.
// Captured off a real pty running `bash -i` under this machine's own atuin config (probe #475):
// **the up-arrow sends `?1049h` and Escape sends `?1049l`**, a full-screen search on the alternate
// screen, with no `3J` anywhere. So every history search takes herdr's ring off the node and gives
// it back, which reaches this client as a discard and then the whole ring arriving at once.
//
// Both reports land on the same reader: one who is *parked* rather than following. That is not a
// contrived state and it is where scrolling down puts you — on a pane whose caret sits above the
// end of its record, the bottom of the grid is below the live edge, so a hand that travels to the
// end of the pane lands there and only typing re-arms the follow (#428,
// `aReaderAtTheBottomIsNotDraggedBackToTheCaret`). `docker compose pull` is exactly that pane: the
// caret parks above a block it redraws in place.
private val DESK = 1600.dp to 900.dp

private const val GRID_ROWS = 200
private const val CARET = 120
private const val RING = 300

// Enough notches to walk the whole surface at three rows a notch, twice over.
private const val NOTCHES_TO_THE_END = 400

private fun history(fromTop: Int, count: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = fromTop,
    rows = (fromTop until fromTop + count).map { RowDiff(it, listOf(Run(0, "history row $it"))) },
    totalRows = fromTop + count,
    complete = true,
    capped = false,
)

// What the node publishes the moment a harness owns the screen: nothing at or past the client's
// end, which is it saying it no longer vouches for the shell era rather than a tail that grew by no
// rows. The same discriminator `ScrollbackStore.apply` reads.
private fun ringTakenAway(end: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = end,
    rows = emptyList(),
    totalRows = 0,
    complete = true,
    capped = false,
)

// Rows only ever enter the ring because the pane wrote lines, and the write is what this surface
// recomposes on — `pane.cursor` is snapshot state and the ring is not. So a scrollback frame in a
// test is delivered the way the node delivers one: behind the grid frame that produced it.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wrote(pane: PaneState, scrollback: ServerMsg.Scrollback) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(GRID_ROWS - 1, listOf(Run(0, "output ${scrollback.totalRows}")))),
            cursor = Cursor(scrollback.totalRows % 7, CARET, true),
            links = emptyList(),
        ),
    )
    pane.applyScrollback(scrollback)
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.paneWithHistory(): Pair<PaneState, PaneSession> {
    val pane = Phone.filled(rows = GRID_ROWS, caretRow = CARET)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = DESK.first, height = DESK.second)
    wrote(pane, history(0, RING))
    assertEquals(RING, pane.scrollback.historyRows, "the ring has to be held here, or nothing is tested")
    assertTrue(
        session.view.band.floor > 0f,
        "the caret has to hold the live edge off the bottom of the grid, or nothing is tested",
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
private fun ComposeUiTest.parkedAtTheBottom(): Pair<PaneState, PaneSession> {
    val (pane, session) = paneWithHistory()
    wheelToTheBottom()
    assertEquals(0f, session.view.scrollY, 0.01f, "never reached the bottom of the grid")
    assertTrue(
        !session.view.following,
        "the bottom of this grid is not its live edge, or nothing is tested",
    )
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
class SearchingTheShellsHistoryTest {
    // A ring the node stopped vouching for and then vouched for again is the same rows arriving
    // twice, not three hundred rows the pane produced. The carry is anchored on the top of the
    // surface, so it read the second delivery as growth — and the discard before it had already
    // clamped the position away at zero, so the two moves cannot cancel.
    @Test
    fun a_history_search_does_not_carry_the_reader_back_by_the_whole_ring() = runComposeUiTest {
        val (pane, session) = parkedAtTheBottom()
        val view = session.view

        // Up: atuin takes the alternate screen and the node stops vouching for the ring.
        wrote(pane, ringTakenAway(RING))
        assertEquals(0, pane.scrollback.historyRows, "the ring was not discarded")

        // Escape: the main screen comes back and the node vouches for it again.
        wrote(pane, history(0, RING))
        assertEquals(RING, pane.scrollback.historyRows, "the ring did not come back")

        assertEquals(
            0f,
            view.scrollY,
            0.01f,
            "a history search carried the reader ${view.scrollY / session.grid.cellHeight} rows " +
                "off the bottom they were parked on",
        )
    }

    // The same rows arriving twice under a reader parked *in* history, which is the other place the
    // discard's clamp destroys a position the refill then cannot put back.
    @Test
    fun a_history_search_does_not_move_a_reader_parked_in_history() = runComposeUiTest {
        val (pane, session) = paneWithHistory()
        val view = session.view
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            repeat(20) { scroll(-1f, ScrollWheel.Vertical) }
        }
        waitForIdle()
        assertTrue(!view.following, "the hand has to have left the live edge, or nothing is tested")
        val parked = view.scrollY
        assertTrue(parked > view.band.floor, "the hand went nowhere")

        wrote(pane, ringTakenAway(RING))
        wrote(pane, history(0, RING))

        assertEquals(parked, view.scrollY, 0.01f, "the ring going and coming back moved the reader")
    }

    // The operator's other half: output the pane really did produce, under a reader who has
    // scrolled to the end of it. Rows entering the ring do not move the bottom of the grid, and a
    // reader who asked for the bottom of the grid is asking for the end of the pane — carrying
    // them up by everything that arrives is the terminal scrolling up under them.
    @Test
    fun output_does_not_carry_a_reader_off_the_bottom_they_scrolled_to() = runComposeUiTest {
        val (pane, session) = parkedAtTheBottom()

        repeat(3) { batch -> wrote(pane, history(RING + batch * 12, 12)) }

        assertEquals(
            0f,
            session.view.scrollY,
            0.01f,
            "thirty-six rows of output carried the reader " +
                "${session.view.scrollY / session.grid.cellHeight} rows up the pane",
        )
    }

    // And the reader the carry is actually for is still carried: parked in history, with rows the
    // pane really did produce arriving between their rows and the live grid.
    @Test
    fun output_still_carries_a_reader_parked_in_history() = runComposeUiTest {
        val (pane, session) = paneWithHistory()
        val view = session.view
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            repeat(20) { scroll(-1f, ScrollWheel.Vertical) }
        }
        waitForIdle()
        val parked = view.scrollY

        wrote(pane, history(RING, 12))

        assertEquals(
            parked + 12 * session.grid.cellHeight,
            view.scrollY,
            0.01f,
            "twelve rows scrolling off the live grid must move a parked reader by twelve rows",
        )
    }
}
