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
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The operator, on a phone, on 0.1.64: *"I quit Claude, see the terminal, it moves to the text
// entry line, then terminal jumps back up to a previous `top` commands output and I need to
// manually scroll up a little then down again to get to the terminal entry line"*.
//
// The numbers here are probe #498, off a real herdr with a real node: a harness taking the
// alternate screen and giving it back hands this client **the same shell-era rows three times**,
// each delivery based exactly on the client's own end. `seq 1 200` built a 163-row ring, and the
// client was sent 162 rows at `from_top: 163`, then the discard at `from_top: 326`, then 164 rows
// at `from_top: 326` — the last of them carrying `"[15:12:36 dbrain@comingclean tmp]$ seq 1 200"`
// as its first row, which is row 0 of the ring it was already holding.
//
// Nothing in the message says so. A refill lands where a tail would land, so `ScrollbackStore`
// counts it as the pane having produced its whole shell era over again, and `carryHistory` moves
// a parked reader up by every row of it — into the era itself, which is where the operator's `top`
// output is.
private const val RING = 163
private const val REDELIVERED = 162
private const val REFILLED = 164
private const val GRID_ROWS = 40
private const val CARET = 30

// Where a shell's prompt sits on the pane a harness was started from and comes back to: a few rows
// down a grid the desk made tall, with the whole rest of it blank tail.
private const val PROMPT_AT = 6

private val PHONE = 411.dp to 914.dp

// The ring the shell era left behind, at whatever base the node is serving it from this time. The
// text is a function of the row's place in the era rather than of its index, because that is the
// whole claim: three deliveries, one set of rows.
private fun era(fromTop: Int, count: Int, era: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = fromTop,
    rows = (0 until count).map {
        RowDiff(fromTop + it, listOf(Run(0, "shell era row $it")))
    },
    totalRows = count,
    complete = false,
    capped = true,
    era = era,
)

// What the node sends the moment a harness owns the screen: nothing, at the end of what it last
// served. Measured at `from_top: 326, total_rows: 0` with no rows.
private fun ringTakenAway(end: Int, era: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = end,
    rows = emptyList(),
    totalRows = 0,
    complete = false,
    capped = true,
    era = era,
)

// Rows only ever enter the ring because the pane wrote lines, and the write is what this surface
// recomposes on — `pane.cursor` is snapshot state and `ScrollbackStore` is not. So every frame here
// is delivered the way the node delivers one, behind a grid frame that moved the caret.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wrote(
    pane: PaneState,
    wrote: Int,
    scrollback: ServerMsg.Scrollback,
    row: Int = GRID_ROWS - 1,
    caret: Int = CARET,
) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, "[dbrain@comingclean]$ output $wrote")))),
            cursor = Cursor(wrote % 7, caret, true),
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
    phoneTerminal(pane, session, width = PHONE.first, height = PHONE.second)
    wrote(pane, 0, era(0, RING, era = 0))
    assertEquals(RING, pane.scrollback.historyRows, "the ring has to be held here, or nothing is tested")
    return pane to session
}

// A hand on a phone, reading the pane rather than riding its live edge. Every gesture that is not
// a landing on the floor parks a reader — a drag, a fling, a pinch — and a reader of an agent's
// screen has made dozens of them.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.parkedInTheRecord(): Pair<PaneState, PaneSession> {
    val (pane, session) = paneWithHistory()
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        repeat(20) { scroll(-1f, ScrollWheel.Vertical) }
    }
    waitForIdle()
    assertTrue(!session.view.following, "the hand has to have left the live edge, or nothing is tested")
    return pane to session
}

// The whole cycle, as probe #498 measured it: the ring re-delivered on the way in, taken away for
// as long as the harness holds the screen, and re-delivered again on the way out.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aHarnessTakesTheScreenAndGivesItBack(pane: PaneState) {
    wrote(pane, 1, era(RING, REDELIVERED, era = 1))
    wrote(pane, 2, ringTakenAway(RING + REDELIVERED, era = 2))
    wrote(pane, 3, era(RING + REDELIVERED, REFILLED, era = 3))
}

// The pane a shell is left on when a harness gives the screen back: a few rows of record at the
// top, the prompt on the last of them, and the whole rest of the grid blank tail. That is what a
// terminal Claude was started from looks like the moment it exits — and what `top` leaves too,
// once the shell has printed a prompt under it.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aShellWithItsPromptNearTheTop(caret: Int): Pair<PaneState, PaneSession> {
    val pane = Phone.shell(rows = GRID_ROWS, caretRow = caret)
    val session = PaneSession(Phone.PANE)
    val bars = phoneTerminal(pane, session, width = PHONE.first, height = PHONE.second)
    // The keyboard is up in every one of the operator's reports, and it is what makes the
    // rectangle short enough for the clamp below to bite.
    bars.value = Phone.KEYBOARD
    waitForIdle()
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 3)
    waitForIdle()
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
class QuittingAnAgentTest {
    // The operator, on the same report, having touched nothing at all: *"ctrl+d twice to quit,
    // waited, jumped up, stayed up"*, and again after a `top`: *"waited - bounced up never bounced
    // back down again"*. A follower is not carried by history — `carryHistory` skips it — so the
    // ring coming back had to be moving them another way, and it was: **it was redrawing the pane
    // at a different size.**
    //
    // The fill has the rows above a short grid to spare, so a pane that has a ring is drawn at
    // fit-width with history above it and a pane with none is magnified to fill the height. Those
    // are two sizes for one pane, and a harness picks between them by starting: it takes herdr's
    // ring for as long as it holds the screen and hands it back on exit (#498). Measured on the
    // phone with the keyboard up: the cell went **19 pixels to 10** and back, seconds after a
    // `claude` the operator had already quit, with the row they were reading half a screen from
    // where it had been.
    @Test
    fun a_ring_going_away_and_coming_back_does_not_redraw_the_pane_at_another_size() =
        runComposeUiTest {
            val (pane, session) = aShellWithItsPromptNearTheTop(caret = PROMPT_AT)
            wrote(pane, 1, era(0, RING, era = 0), row = PROMPT_AT, caret = PROMPT_AT)
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 3)
            waitForIdle()
            assertTrue(session.view.following, "a pane nobody has touched follows its own output")
            val cell = session.grid.cellHeight
            val caretAt = rowTop(pane, session, pane.cursor.row)

            // A harness takes the screen, and gives it back with the same rows in it.
            wrote(pane, 2, ringTakenAway(RING, era = 1), row = PROMPT_AT, caret = PROMPT_AT)
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 3)
            waitForIdle()
            // While it holds the screen, and not only once it has given it back: this is the whole
            // of what the operator is looking at for as long as the harness runs.
            assertEquals(
                cell,
                session.grid.cellHeight,
                0.01f,
                "the harness took the ring and the pane was redrawn at ${session.grid.cellHeight} " +
                    "pixels a row against $cell",
            )
            assertEquals(
                caretAt,
                rowTop(pane, session, pane.cursor.row),
                "the harness took the ring and the row being typed into left the bottom of the " +
                    "screen for $caretAt — the operator: *\"if I didn't manually scroll up I want " +
                    "it to follow the bottom\"*",
            )

            wrote(pane, 3, era(RING, RING, era = 2), row = PROMPT_AT, caret = PROMPT_AT)
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 3)
            waitForIdle()

            assertEquals(
                cell,
                session.grid.cellHeight,
                0.01f,
                "the ring went away and came back and the pane is drawn at another size: " +
                    "${session.grid.cellHeight} pixels a row against $cell",
            )
            assertEquals(
                caretAt,
                rowTop(pane, session, pane.cursor.row),
                "the row being typed into moved from $caretAt to " +
                    "${rowTop(pane, session, pane.cursor.row)} across a harness that came and went",
            )
        }

    @Test
    fun a_ring_delivered_a_second_time_does_not_carry_a_parked_reader_into_it() = runComposeUiTest {
        val (pane, session) = parkedInTheRecord()
        val parked = session.view.scrollY

        aHarnessTakesTheScreenAndGivesItBack(pane)

        assertEquals(
            parked,
            session.view.scrollY,
            0.01f,
            "quitting the harness carried the reader " +
                "${(session.view.scrollY - parked) / session.grid.cellHeight} rows up the pane, " +
                "into the shell era it was handed a second time",
        )
    }

    @Test
    fun a_ring_delivered_a_second_time_leaves_a_follower_on_the_live_edge() = runComposeUiTest {
        val (pane, session) = paneWithHistory()
        assertTrue(session.view.following, "a pane nobody has touched follows its own output")
        val resting = session.view.scrollY

        aHarnessTakesTheScreenAndGivesItBack(pane)

        assertTrue(session.view.following, "the follower stopped following")
        assertEquals(resting, session.view.scrollY, 0.01f, "the follower was taken off the live edge")
    }
}
